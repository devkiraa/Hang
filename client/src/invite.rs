use serde::{Deserialize, Serialize};
use url::Url;
use urlencoding::{decode, encode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteSignal {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct InviteLink {
    pub room_id: String,
    pub passcode: Option<String>,
    pub file_name: Option<String>,
}

pub fn build_invite_url(room_id: &str, passcode: Option<&str>, file_name: Option<&str>) -> String {
    let mut params = vec![format!("room={}", encode(room_id))];
    if let Some(code) = passcode.filter(|c| !c.is_empty()) {
        params.push(format!("code={}", encode(code)));
    }
    if let Some(name) = file_name.filter(|n| !n.is_empty()) {
        params.push(format!("file={}", encode(name)));
    }
    let query = params.join("&");
    format!("hang://join?{}", query)
}

pub fn parse_invite_url(input: &str) -> Option<InviteLink> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = if trimmed.starts_with("hang://") {
        trimmed.to_string()
    } else if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("hang://join?{}", trimmed)
    };

    let url = Url::parse(&normalized).ok()?;
    let mut room_id = None;
    let mut passcode = None;
    let mut file_name = None;

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "room" => room_id = decode(&value).ok().map(|v| v.into_owned()),
            "code" => passcode = decode(&value).ok().map(|v| v.into_owned()),
            "file" => file_name = decode(&value).ok().map(|v| v.into_owned()),
            _ => {}
        }
    }

    let room_id = room_id?;
    Some(InviteLink {
        room_id,
        passcode,
        file_name,
    })
}
