use axum::{http::header, response::IntoResponse, Json};

pub fn metadata() -> serde_json::Value {
    let commit = env!("FRAYD_BUILD_COMMIT");
    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": if commit == "unknown" { None } else { Some(commit) },
        "commits_since_tag": env!("FRAYD_BUILD_COMMITS_SINCE_TAG").parse::<u64>().ok(),
        "dirty": env!("FRAYD_BUILD_DIRTY").parse::<bool>().ok(),
    })
}

pub async fn handler() -> impl IntoResponse {
    ([(header::CACHE_CONTROL, "no-store")], Json(metadata()))
}

#[cfg(test)]
mod tests {
    /// build.rs output must parse into the documented shape: a full lowercase commit hash or null,
    /// a tag distance or null, and a dirty flag or null.
    #[tokio::test]
    async fn endpoint_is_uncached_and_reports_well_formed_identity() {
        use axum::response::IntoResponse;
        let response = super::handler().await.into_response();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.keys().map(String::as_str).collect::<Vec<_>>(), ["commit", "commits_since_tag", "dirty", "version"]);
        assert!(value["version"].is_string());
        assert!(value["commit"].is_null() || value["commit"].as_str().is_some_and(|commit|
            commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))));
        assert!(value["commits_since_tag"].is_null() || value["commits_since_tag"].is_u64());
        assert!(value["dirty"].is_null() || value["dirty"].is_boolean());
    }
}
