use reqwest::Client;

pub fn build_runtime_proxy_client_with_timeouts(service_key: &str, timeout: u64, conn_timeout_secs: u64) -> Client{
    Client::new()
}
