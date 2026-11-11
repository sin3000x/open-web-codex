//! Static delivery of the built browser application.
//!
//! Vite writes only content-hashed files into `assets/`, so those responses
//! are immutable. The HTML shell is the single mutable pointer to them and
//! must be revalidated on every load.

use std::path::Path;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Response};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;

const HASHED_ASSETS_DIRECTORY: &str = "assets";
const IMMUTABLE: HeaderValue = HeaderValue::from_static("public, max-age=31536000, immutable");
const REVALIDATE: HeaderValue = HeaderValue::from_static("no-cache");

pub fn service(web_dist: &Path) -> Router {
    let hashed_assets = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            CACHE_CONTROL,
            |response: &Response<_>| response.status().is_success().then_some(IMMUTABLE),
        ))
        .service(ServeDir::new(web_dist.join(HASHED_ASSETS_DIRECTORY)));
    let shell = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            CACHE_CONTROL,
            |response: &Response<_>| is_html(response).then_some(REVALIDATE),
        ))
        .service(ServeDir::new(web_dist).fallback(ServeFile::new(web_dist.join("index.html"))));
    Router::new()
        .nest_service("/assets", hashed_assets)
        .fallback_service(shell)
        .layer(CompressionLayer::new())
}

fn is_html<B>(response: &Response<B>) -> bool {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::body::Body;
    use axum::http::header::{ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn dist() -> tempfile::TempDir {
        let dist = tempfile::tempdir().unwrap();
        fs::create_dir(dist.path().join("assets")).unwrap();
        fs::write(dist.path().join("index.html"), "<!doctype html>").unwrap();
        fs::write(
            dist.path().join("assets/app-abc123.js"),
            "export const value = 1;\n".repeat(64),
        )
        .unwrap();
        fs::write(dist.path().join("brand-logo.png"), [0x89, b'P', b'N', b'G']).unwrap();
        dist
    }

    async fn get(dist: &tempfile::TempDir, path: &str) -> axum::http::Response<Body> {
        let request = Request::builder()
            .uri(path)
            .header(ACCEPT_ENCODING, "gzip")
            .body(Body::empty())
            .unwrap();
        super::service(dist.path()).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn hashed_assets_are_immutable_and_compressed() {
        let dist = dist();
        let response = get(&dist, "/assets/app-abc123.js").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
    }

    #[tokio::test]
    async fn missing_hashed_asset_is_an_uncached_not_found() {
        let dist = dist();
        let response = get(&dist, "/assets/app-stale.js").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(response.headers().get(CACHE_CONTROL).is_none());
    }

    #[tokio::test]
    async fn html_shell_and_route_fallback_are_revalidated() {
        let dist = dist();
        for path in ["/", "/index.html", "/tasks/42"] {
            let response = get(&dist, path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert_eq!(response.headers()[CACHE_CONTROL], "no-cache", "{path}");
        }
    }

    #[tokio::test]
    async fn unhashed_public_files_keep_validator_based_caching() {
        let dist = dist();
        let response = get(&dist, "/brand-logo.png").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(CACHE_CONTROL).is_none());
        assert!(response.headers().get(CONTENT_ENCODING).is_none());
    }
}
