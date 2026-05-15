use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, NaiveDate, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time,
};
use url::Url;

use crate::{
    config::GoogleConfig,
    model::{Calendar, CalendarSource, Event, NewEvent, ProviderCapabilities, TimeRange},
    provider::CalendarProvider,
};

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_CALENDAR_API: &str = "https://www.googleapis.com/calendar/v3";
const GOOGLE_SCOPE: &str = "https://www.googleapis.com/auth/calendar";
const CALLBACK_PATH: &str = "/oauth/google/callback";

#[derive(Debug, Clone)]
pub struct GoogleProvider {
    config: GoogleConfig,
    tokens: Option<GoogleTokenSet>,
    provider_id: String,
}

impl GoogleProvider {
    pub fn new(config: GoogleConfig) -> Self {
        Self {
            config,
            tokens: None,
            provider_id: "google".to_string(),
        }
    }

    pub fn with_tokens(config: GoogleConfig, tokens: GoogleTokenSet, provider_id: String) -> Self {
        Self {
            config,
            tokens: Some(tokens),
            provider_id,
        }
    }

    pub fn browser_auth_url(
        &self,
        redirect_uri: &str,
        state: &str,
        code_challenge: &str,
    ) -> Result<Url> {
        let mut url = Url::parse(GOOGLE_AUTH_URL)?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", GOOGLE_SCOPE)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("include_granted_scopes", "true")
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);

        Ok(url)
    }

    pub async fn login_with_browser(&self) -> Result<GoogleTokenSet> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("failed to bind local OAuth callback listener")?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let state = random_url_safe(24);
        let code_verifier = random_url_safe(64);
        let code_challenge = pkce_challenge(&code_verifier);
        let url = self.browser_auth_url(&redirect_uri, &state, &code_challenge)?;

        open::that(url.as_str())?;
        let code = wait_for_callback(listener, &state).await?;
        self.exchange_code_for_tokens(&code, &redirect_uri, &code_verifier)
            .await
    }

    pub async fn refresh_tokens(&self, tokens: &GoogleTokenSet) -> Result<GoogleTokenSet> {
        let refresh_token = tokens.refresh_token.as_deref().context(
            "Google token file does not include a refresh_token; press L to log in again",
        )?;

        let mut form = vec![
            ("client_id", self.config.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ];

        if !self.config.client_secret.trim().is_empty() {
            form.push(("client_secret", self.config.client_secret.as_str()));
        }

        let response = reqwest::Client::new()
            .post(GOOGLE_TOKEN_URL)
            .form(&form)
            .send()
            .await
            .context("failed to send Google token refresh request")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read Google token refresh response")?;

        if !status.is_success() {
            bail!("Google token refresh failed with {status}: {body}");
        }

        GoogleTokenSet::from_token_response(&body, tokens.refresh_token.clone())
    }

    async fn exchange_code_for_tokens(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<GoogleTokenSet> {
        let mut form = vec![
            ("client_id", self.config.client_id.as_str()),
            ("code", code),
            ("code_verifier", code_verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ];

        if !self.config.client_secret.trim().is_empty() {
            form.push(("client_secret", self.config.client_secret.as_str()));
        }

        let response = reqwest::Client::new()
            .post(GOOGLE_TOKEN_URL)
            .form(&form)
            .send()
            .await
            .context("failed to send Google token exchange request")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read Google token response")?;

        if !status.is_success() {
            bail!("Google token exchange failed with {status}: {body}");
        }

        GoogleTokenSet::from_token_response(&body, None)
    }

    fn access_token(&self) -> Result<&str> {
        self.tokens
            .as_ref()
            .map(|tokens| tokens.access_token.as_str())
            .context("Google is not logged in; press L to authenticate")
    }

    pub async fn account_summary(&self) -> Result<GoogleAccountSummary> {
        let calendars = self.fetch_calendar_list().await?;
        let primary = calendars
            .iter()
            .find(|calendar| calendar.primary)
            .or_else(|| calendars.first())
            .context("Google account did not return any calendars")?;

        Ok(GoogleAccountSummary {
            label: primary.summary.clone(),
            primary_calendar_id: Some(primary.id.clone()),
        })
    }

    async fn fetch_calendar_list(&self) -> Result<Vec<GoogleCalendarListEntry>> {
        let response = reqwest::Client::new()
            .get(format!("{GOOGLE_CALENDAR_API}/users/me/calendarList"))
            .bearer_auth(self.access_token()?)
            .send()
            .await
            .context("failed to fetch Google calendar list")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read Google calendar list response")?;

        if !status.is_success() {
            bail!("Google calendar list fetch failed with {status}: {body}");
        }

        let response: CalendarListResponse =
            serde_json::from_str(&body).context("failed to parse Google calendar list")?;
        Ok(response.items)
    }
}

#[derive(Debug, Clone)]
pub struct GoogleAccountSummary {
    pub label: String,
    pub primary_calendar_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenStore {
    #[serde(default)]
    pub google: Vec<StoredGoogleAccount>,
}

impl TokenStore {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read tokens from {}", path.display()))?;
        let value: toml::Value = toml::from_str(&contents)
            .with_context(|| format!("failed to parse tokens from {}", path.display()))?;

        if value.get("access_token").is_some() {
            let tokens: GoogleTokenSet = value
                .try_into()
                .context("failed to parse legacy single-account Google tokens")?;
            return Ok(Self {
                google: vec![StoredGoogleAccount {
                    id: "legacy".to_string(),
                    label: "Google account".to_string(),
                    primary_calendar_id: None,
                    tokens,
                }],
            });
        }

        value
            .try_into()
            .context("failed to parse multi-account token store")
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        fs::write(path, contents).context("failed to write token store")
    }

    pub fn upsert_google_account(&mut self, account: StoredGoogleAccount) {
        if let Some(primary_calendar_id) = account.primary_calendar_id.as_deref() {
            if let Some(existing) = self.google.iter_mut().find(|existing| {
                existing.primary_calendar_id.as_deref() == Some(primary_calendar_id)
            }) {
                *existing = account;
                return;
            }
        }

        self.google.push(account);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredGoogleAccount {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub primary_calendar_id: Option<String>,
    pub tokens: GoogleTokenSet,
}

impl StoredGoogleAccount {
    pub fn new(label: String, primary_calendar_id: Option<String>, tokens: GoogleTokenSet) -> Self {
        Self {
            id: new_google_account_id(),
            label,
            primary_calendar_id,
            tokens,
        }
    }

    pub fn provider_id(&self) -> String {
        format!("google:{}", self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleTokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub scope: String,
    pub token_type: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

impl GoogleTokenSet {
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|expires_at| expires_at <= Utc::now())
            .unwrap_or(true)
    }

    fn from_token_response(body: &str, existing_refresh_token: Option<String>) -> Result<Self> {
        let response: GoogleTokenResponse =
            serde_json::from_str(body).context("failed to parse Google token response")?;

        Ok(Self {
            access_token: response.access_token,
            refresh_token: response.refresh_token.or(existing_refresh_token),
            scope: response.scope.unwrap_or_else(|| GOOGLE_SCOPE.to_string()),
            token_type: response.token_type,
            expires_at: Some(
                Utc::now() + chrono::Duration::seconds(response.expires_in as i64 - 60),
            ),
            expires_in: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    token_type: String,
}

#[derive(Debug, Deserialize)]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<GoogleCalendarListEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarListEntry {
    id: String,
    summary: String,
    #[serde(default)]
    background_color: Option<String>,
    #[serde(default)]
    access_role: String,
    #[serde(default)]
    deleted: bool,
    #[serde(default)]
    primary: bool,
}

#[derive(Debug, Deserialize)]
struct EventsResponse {
    #[serde(default)]
    items: Vec<GoogleEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleEvent {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    status: Option<String>,
    start: GoogleEventTime,
    end: GoogleEventTime,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleEventTime {
    #[serde(default)]
    date_time: Option<DateTime<Utc>>,
    #[serde(default)]
    date: Option<NaiveDate>,
}

impl GoogleEventTime {
    fn as_utc(&self) -> DateTime<Utc> {
        self.date_time
            .or_else(|| {
                self.date
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|date_time| date_time.and_utc())
            })
            .unwrap_or_else(Utc::now)
    }
}

async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    let callback = time::timeout(Duration::from_secs(300), async {
        let (mut stream, _) = listener.accept().await?;
        let mut buffer = [0_u8; 4096];
        let bytes_read = stream.read(&mut buffer).await?;
        let request = String::from_utf8_lossy(&buffer[..bytes_read]);
        let request_line = request
            .lines()
            .next()
            .ok_or_else(|| anyhow!("OAuth callback request was empty"))?;
        let callback_url = parse_callback_request_line(request_line)?;
        let result = parse_callback_url(&callback_url, expected_state);

        let response_body = if result.is_ok() {
            "tuical Google login complete. You can close this tab."
        } else {
            "tuical Google login failed. Return to the terminal for details."
        };
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        stream.write_all(response.as_bytes()).await?;

        result
    })
    .await
    .context("timed out waiting for Google OAuth callback")??;

    Ok(callback)
}

fn parse_callback_request_line(request_line: &str) -> Result<Url> {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();

    ensure!(
        method == "GET",
        "OAuth callback used unexpected method {method}"
    );
    ensure!(
        path.starts_with(CALLBACK_PATH),
        "OAuth callback used unexpected path {path}"
    );

    Url::parse(&format!("http://127.0.0.1{path}")).context("failed to parse OAuth callback URL")
}

fn parse_callback_url(url: &Url, expected_state: &str) -> Result<String> {
    let mut code = None;
    let mut state = None;
    let mut error = None;

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = error {
        bail!("Google OAuth returned error: {error}");
    }

    ensure!(
        state.as_deref() == Some(expected_state),
        "OAuth state mismatch"
    );

    code.ok_or_else(|| anyhow!("Google OAuth callback did not include an authorization code"))
}

fn random_url_safe(bytes: usize) -> String {
    let mut random = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut random);
    URL_SAFE_NO_PAD.encode(random)
}

fn pkce_challenge(code_verifier: &str) -> String {
    let digest = Sha256::digest(code_verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn new_google_account_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("acct-{millis}")
}

#[async_trait]
impl CalendarProvider for GoogleProvider {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::google()
    }

    async fn list_calendars(&self) -> Result<Vec<Calendar>> {
        Ok(self
            .fetch_calendar_list()
            .await?
            .into_iter()
            .filter(|calendar| !calendar.deleted)
            .map(|calendar| Calendar {
                id: format!("{}:{}", self.provider_id, calendar.id),
                remote_id: calendar.id,
                provider_id: self.provider_id.clone(),
                source: CalendarSource::Google,
                name: calendar.summary,
                color: calendar
                    .background_color
                    .unwrap_or_else(|| "blue".to_string()),
                read_only: !matches!(calendar.access_role.as_str(), "owner" | "writer"),
            })
            .collect())
    }

    async fn list_events(&self, calendar_id: &str, range: TimeRange) -> Result<Vec<Event>> {
        let encoded_calendar_id = utf8_percent_encode(calendar_id, NON_ALPHANUMERIC).to_string();
        let response = reqwest::Client::new()
            .get(format!(
                "{GOOGLE_CALENDAR_API}/calendars/{encoded_calendar_id}/events"
            ))
            .bearer_auth(self.access_token()?)
            .query(&[
                ("singleEvents", "true"),
                ("orderBy", "startTime"),
                ("timeMin", &range.starts_at.to_rfc3339()),
                ("timeMax", &range.ends_at.to_rfc3339()),
            ])
            .send()
            .await
            .with_context(|| format!("failed to fetch Google events for {calendar_id}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("failed to read Google events response")?;

        if !status.is_success() {
            bail!("Google events fetch failed for {calendar_id} with {status}: {body}");
        }

        let response: EventsResponse =
            serde_json::from_str(&body).context("failed to parse Google events")?;

        Ok(response
            .items
            .into_iter()
            .filter(|event| event.status.as_deref() != Some("cancelled"))
            .map(|event| Event {
                id: event.id,
                provider_id: self.provider_id.clone(),
                calendar_id: calendar_id.to_string(),
                title: event.summary.unwrap_or_else(|| "(untitled)".to_string()),
                description: event.description,
                location: event.location,
                starts_at: event.start.as_utc(),
                ends_at: event.end.as_utc(),
                all_day: event.start.date.is_some(),
                color: "blue".to_string(),
            })
            .collect())
    }

    async fn create_event(&self, _calendar_id: &str, _event: NewEvent) -> Result<Event> {
        bail!("Google event creation is not implemented yet")
    }

    async fn update_event(&self, _event: Event) -> Result<Event> {
        bail!("Google event update is not implemented yet")
    }

    async fn move_event(
        &self,
        _event_id: &str,
        _from_calendar_id: &str,
        _to_calendar_id: &str,
    ) -> Result<Event> {
        bail!("Google event move is not implemented yet")
    }
}
