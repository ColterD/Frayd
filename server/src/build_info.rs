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
    #[tokio::test]
    async fn endpoint_is_uncached_and_matches_binary_identity() {
        use axum::response::IntoResponse;
        let response = super::handler().await.into_response();
        assert_eq!(response.headers()["cache-control"], "no-store");
        let body = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), super::metadata());
        assert_eq!(super::metadata()["version"], env!("CARGO_PKG_VERSION"));
    }
}
