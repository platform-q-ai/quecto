use super::*;

fn no_threads() -> std::io::Result<tokio::runtime::Runtime> {
    Err(std::io::Error::from_raw_os_error(libc::EAGAIN))
}

/// A test server for every provider, so OpenAI's endpoint is configured;
/// nothing listens there, and nothing is ever requested from it.
fn endpoints() -> OAuthEndpoints {
    OAuthEndpoints::new(Some("http://127.0.0.1:1".to_string()))
}

#[test]
fn a_process_that_cannot_start_a_thread_is_unavailable_not_aborted() {
    let flow = OAuthDeviceCodeFlow::on_runtimes(endpoints(), no_threads);
    match flow.request(LoginProvider::OpenAi) {
        Err(DeviceCodeError::Unavailable(reason)) => {
            assert!(reason.contains("temporarily unavailable"), "{reason}");
        }
        other => panic!("expected an unavailable flow, got {other:?}"),
    }
}

#[test]
fn an_unsupported_provider_is_refused_before_any_runtime_is_built() {
    let flow = OAuthDeviceCodeFlow::on_runtimes(OAuthEndpoints::new(None), no_threads);
    assert_eq!(
        flow.request(LoginProvider::Xai),
        Err(DeviceCodeError::Unsupported)
    );
}

#[tokio::test]
async fn a_call_from_inside_a_runtime_is_unavailable_not_a_panic() {
    let flow = OAuthDeviceCodeFlow::new(endpoints());
    assert_eq!(
        flow.request(LoginProvider::OpenAi),
        Err(DeviceCodeError::Unavailable(
            "called from inside an async runtime".to_string()
        ))
    );
}
