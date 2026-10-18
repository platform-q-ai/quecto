use std::sync::{Arc, Mutex};

use super::*;
use crate::application::auth::ports::{DeviceCodeError, DeviceCodeFlow, DeviceCodeGrant};

/// Answers every request with `answer`, recording who asked.
struct FakeFlow {
    asked: Mutex<Vec<LoginProvider>>,
    answer: Result<DeviceCodeGrant, DeviceCodeError>,
}

impl FakeFlow {
    fn answering(answer: Result<DeviceCodeGrant, DeviceCodeError>) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answer,
        })
    }
}

impl DeviceCodeFlow for FakeFlow {
    fn request(&self, provider: LoginProvider) -> Result<DeviceCodeGrant, DeviceCodeError> {
        self.asked.lock().unwrap().push(provider);
        self.answer.clone()
    }
}

fn grant() -> DeviceCodeGrant {
    DeviceCodeGrant {
        verification_uri: "https://example.test/device".to_string(),
        user_code: "ABCD-1234".to_string(),
    }
}

fn start(flow: &Arc<FakeFlow>) -> StartDeviceCodeLogin {
    StartDeviceCodeLogin::new(Arc::clone(flow) as Arc<dyn DeviceCodeFlow>)
}

#[test]
fn a_granted_request_tells_the_user_where_to_go_and_what_to_enter() {
    let flow = FakeFlow::answering(Ok(grant()));
    let shown = start(&flow).execute("openai").unwrap();
    assert_eq!(
        shown,
        DeviceCodeInstructions {
            verification_uri: "https://example.test/device".to_string(),
            user_code: "ABCD-1234".to_string(),
        }
    );
    assert_eq!(*flow.asked.lock().unwrap(), [LoginProvider::OpenAi]);
}

#[test]
fn an_unknown_provider_is_refused_without_asking_anyone() {
    let flow = FakeFlow::answering(Ok(grant()));
    assert_eq!(
        start(&flow).execute("weird"),
        Err(LoginError::UnknownProvider("weird".to_string()))
    );
    assert!(flow.asked.lock().unwrap().is_empty());
}

#[test]
fn a_provider_not_offering_the_flow_is_refused_without_asking_anyone() {
    // Even a flow that would grant one (a test server) is never asked.
    let flow = FakeFlow::answering(Ok(grant()));
    for provider in [LoginProvider::Xai, LoginProvider::Anthropic] {
        assert_eq!(
            start(&flow).execute(provider.id()),
            Err(LoginError::DeviceCodeUnsupported(provider))
        );
    }
    assert!(flow.asked.lock().unwrap().is_empty());
}

#[test]
fn a_flow_without_the_providers_endpoint_is_reported_as_unsupported() {
    let flow = FakeFlow::answering(Err(DeviceCodeError::Unsupported));
    assert_eq!(
        start(&flow).execute("openai"),
        Err(LoginError::DeviceCodeUnsupported(LoginProvider::OpenAi))
    );
}

#[test]
fn a_refused_request_and_a_missing_runtime_keep_their_reasons() {
    let refused = FakeFlow::answering(Err(DeviceCodeError::Request("(500)".to_string())));
    assert_eq!(
        start(&refused).execute("openai"),
        Err(LoginError::DeviceCodeRequest("(500)".to_string()))
    );
    let no_runtime = FakeFlow::answering(Err(DeviceCodeError::Unavailable("EAGAIN".to_string())));
    assert_eq!(
        start(&no_runtime).execute("openai"),
        Err(LoginError::Unavailable("EAGAIN".to_string()))
    );
}
