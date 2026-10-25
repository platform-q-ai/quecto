//! The board service's replies (epic #2705 B4): a handler's reply as the
//! HTTP response the listener sends.
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Response, StatusCode, header};

use super::listener::ServiceReply;

/// A whole reply: its status, its JSON body and headers.
pub(super) fn response(reply: ServiceReply) -> Response<Full<Bytes>> {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(Full::new(Bytes::from(reply.body)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    let json = header::HeaderValue::from_static("application/json");
    headers.insert(header::CONTENT_TYPE, json);
    if let Some(allow) = reply.allow {
        headers.insert(header::ALLOW, header::HeaderValue::from_static(allow));
    }
    if let Some(challenge) = reply.challenge {
        let challenge = header::HeaderValue::from_static(challenge);
        headers.insert(header::WWW_AUTHENTICATE, challenge);
    }
    response
}
