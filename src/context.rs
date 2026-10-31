use crate::config::AppConfig;
use std::time::Duration;

pub struct AppContext {
    pub client: reqwest::Client,
    pub config: AppConfig,
}

impl AppContext {
    pub fn new(config: AppConfig) -> Self {
        let client = reqwest::Client::builder()
            .pool_idle_timeout(Duration::from_secs(60))
            .tcp_nodelay(true)
            // Connect only. A client-wide total timeout would silently cap
            // every longer per-provider budget (the engine wraps each call in
            // Provider::timeout()); callers without an outer budget set
            // RequestBuilder::timeout themselves.
            .connect_timeout(Duration::from_secs(config.settings.timeout))
            .user_agent(format!("search-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build HTTP client");

        Self { client, config }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn settings_timeout_does_not_cap_a_slower_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                .await;
        });
        let mut config = AppConfig::default();
        config.settings.timeout = 1;
        let ctx = AppContext::new(config);
        let resp = ctx.client.get(format!("http://{addr}/")).send().await;
        assert!(resp.is_ok(), "{resp:?}");
    }
}
