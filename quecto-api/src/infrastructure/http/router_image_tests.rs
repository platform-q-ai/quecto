//! #2422: images on HTTP `/prompt` and the WebSocket prompt frame. A child
//! of the router test module, so it shares its gateway double.
use super::*;

/// A 2x2 PNG: signature, IHDR, IEND.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC";

fn connected() -> MockGateway {
    MockGateway {
        connected: true,
        ..MockGateway::default()
    }
}

fn prompt_images(gateway: &MockGateway) -> Vec<Vec<String>> {
    gateway
        .commands
        .lock()
        .unwrap()
        .iter()
        .filter_map(|cmd| match cmd {
            AgentCommand::Prompt { images, .. } => {
                Some(images.iter().map(|i| i.data().to_string()).collect())
            }
            _ => None,
        })
        .collect()
}

async fn serve(gateway: MockGateway) -> std::net::SocketAddr {
    let app = build_router(gateway);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn post_prompt(
    gateway: MockGateway,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let addr = serve(gateway).await;
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/prompt"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap();
    (status, resp.json().await.unwrap_or_default())
}

#[tokio::test]
async fn http_prompt_forwards_its_images() {
    let gateway = connected();
    let body = serde_json::json!({
        "message": "look",
        "images": [{"mimeType": "image/png", "data": PNG}],
    });
    let (status, _) = post_prompt(gateway.clone(), body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(prompt_images(&gateway), [vec![PNG.to_string()]]);
}

#[tokio::test]
async fn http_prompt_takes_images_without_text() {
    let gateway = connected();
    let body = serde_json::json!({
        "message": "",
        "images": [{"mimeType": "image/png", "data": PNG}],
    });
    let (status, _) = post_prompt(gateway.clone(), body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(prompt_images(&gateway).len(), 1);
}

#[tokio::test]
async fn http_prompt_refuses_a_bad_image_with_400_and_the_exact_message() {
    let gateway = connected();
    let body = serde_json::json!({
        "message": "look",
        "images": [{"mimeType": "image/bmp", "data": PNG}],
    });
    let (status, json) = post_prompt(gateway.clone(), body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        json["error"],
        "invalid request: images[0]: mimeType \"image/bmp\" is not allowed; use image/png, image/jpeg, image/gif or image/webp"
    );
    assert!(gateway.commands.lock().unwrap().is_empty());
}

/// A 3.75 MiB image is a 5 MiB base64 body: past axum's 2 MB default,
/// inside the agent's 8 MiB frame cap, on every route that takes images.
#[tokio::test]
async fn image_routes_take_a_body_up_to_the_frame_cap() {
    let bytes = quecto_image::samples::png_with_body(2, 2, quecto_image::MAX_IMAGE_BYTES - 57);
    let data = quecto_image::encode(&bytes);
    for path in ["prompt", "steer", "follow_up"] {
        let gateway = connected();
        let addr = serve(gateway.clone()).await;
        let body = serde_json::json!({
            "message": "big",
            "images": [{"mimeType": "image/png", "data": data}],
        });
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/{path}"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200, "{path}");
        assert_eq!(gateway.commands.lock().unwrap().len(), 1, "{path}");
    }
}

/// Only the image routes are raised: any other keeps axum's 2 MB default.
#[tokio::test]
async fn other_routes_keep_the_default_body_limit() {
    let addr = serve(connected()).await;
    let body = serde_json::json!({"model": "x".repeat(3 * 1024 * 1024)});
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/model"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 413);
}

#[tokio::test]
async fn websocket_prompt_forwards_its_images() {
    let gateway = connected();
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({
            "type": "prompt",
            "message": "look",
            "images": [{"mimeType": "image/png", "data": PNG}],
        }),
    )
    .await;
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(prompt_images(&gateway), [vec![PNG.to_string()]]);
}

#[tokio::test]
async fn websocket_prompt_with_a_bad_image_gets_an_error_frame() {
    let gateway = connected();
    let response = ws_response_for(
        gateway.clone(),
        serde_json::json!({
            "type": "prompt",
            "id": "img-1",
            "message": "look",
            "images": [{"mimeType": "image/png", "data": "not base64"}],
        }),
    )
    .await;
    assert_eq!(response["type"], "response");
    assert_eq!(response["id"], "img-1");
    assert_eq!(response["command"], "prompt");
    assert_eq!(response["success"], false);
    assert_eq!(
        response["error"],
        "invalid request: images[0]: data is not valid standard base64"
    );
    assert!(gateway.commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn http_steer_and_follow_up_carry_and_refuse_images() {
    for path in ["steer", "follow_up"] {
        let gateway = connected();
        let addr = serve(gateway.clone()).await;
        let client = reqwest::Client::new();
        let good = serde_json::json!({
            "message": "",
            "images": [{"mimeType": "image/png", "data": PNG}],
        });
        let resp = client
            .post(format!("http://{addr}/{path}"))
            .json(&good)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200, "{path}");
        let sent = gateway.commands.lock().unwrap().clone();
        assert!(
            matches!(
                sent.as_slice(),
                [AgentCommand::Steer { images, .. } | AgentCommand::FollowUp { images, .. }]
                    if images.len() == 1
            ),
            "{path}: {sent:?}"
        );

        let bad = serde_json::json!({
            "message": "look",
            "images": [{"mimeType": "image/png", "data": "iVBO RW0K"}],
        });
        let resp = client
            .post(format!("http://{addr}/{path}"))
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 400, "{path}");
        let json: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            json["error"],
            "invalid request: images[0]: data is not valid standard base64"
        );
        assert_eq!(gateway.commands.lock().unwrap().len(), 1, "{path}");
    }
}
