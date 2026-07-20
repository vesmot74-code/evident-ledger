//! HttpPaddleClient integration tests against a mock Paddle API.

use evident_ledger::config::AppConfig;
use evident_ledger::paddle::{HttpPaddleClient, PaddleClient};
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn create_customer_reuses_existing_customer_after_conflict() {
    let server = MockServer::start().await;
    let email = "existing@example.com";
    let existing_customer_id = "ctm_01h8x5k8zq7n4m2p9r6s3t0v1w";

    // Real Paddle conflict phrasing observed during sandbox investigation:
    // "customer email conflicts with customer of id ctm_..."
    Mock::given(method("POST"))
        .and(path("/customers"))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "error": {
                "type": "request_error",
                "code": "customer_already_exists",
                "detail": format!(
                    "customer email conflicts with customer of id {existing_customer_id}"
                ),
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/customers"))
        .and(query_param("email", email))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{ "id": existing_customer_id, "email": email }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = AppConfig::test_defaults();
    config.paddle_api_base_url = server.uri();
    let client = HttpPaddleClient::from_config(&config);

    let customer_id = client
        .create_customer(email)
        .await
        .expect("should reuse existing customer after 409");

    assert_eq!(customer_id, existing_customer_id);
}
