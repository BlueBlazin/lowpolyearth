//! Browser `fetch` transport for [`super`]. Every future here holds
//! JavaScript objects, so it is built and polled on the reactor worker via
//! `vearth_platform::rt::spawn_local_on_reactor`.
//!
//! The semantics match the native reqwest client: a whole-request timeout,
//! optional per-chunk stall detection, and cancellation when the attempt is
//! dropped (the `AbortController` aborts the underlying request).

use super::AttemptError;
use crate::DataError;
use std::time::Duration;
use vearth_platform::rt::time::timeout;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[derive(Clone)]
pub(super) struct Client {
    timeout: Duration,
}

pub(super) fn client(timeout: Duration) -> Client {
    Client { timeout }
}

/// Aborts the request when the attempt future is dropped or finishes.
struct AbortOnDrop(web_sys::AbortController);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn js_error(error: JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .unwrap_or_else(|| format!("{error:?}"))
}

async fn send(
    url: &str,
    method: &str,
    range: Option<(u64, u64)>,
    abort: &web_sys::AbortController,
) -> Result<web_sys::Response, String> {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    init.set_mode(web_sys::RequestMode::Cors);
    init.set_signal(Some(&abort.signal()));
    let request = web_sys::Request::new_with_str_and_init(url, &init).map_err(js_error)?;
    if let Some((offset, end)) = range {
        // A single byte range is a CORS-safelisted request header, so it
        // needs no preflight. Browsers request identity encoding for ranges.
        request
            .headers()
            .set("Range", &format!("bytes={offset}-{end}"))
            .map_err(js_error)?;
    }
    let global = js_sys::global().unchecked_into::<web_sys::WorkerGlobalScope>();
    let response = JsFuture::from(global.fetch_with_request(&request))
        .await
        .map_err(js_error)?;
    Ok(response.unchecked_into())
}

/// Reads the body, optionally failing when no chunk arrives for `stall`.
async fn read_body(
    response: &web_sys::Response,
    stall: Option<Duration>,
    limit: Option<usize>,
) -> Result<Vec<u8>, AttemptError> {
    let Some(stall) = stall else {
        let buffer = response
            .array_buffer()
            .map_err(|e| AttemptError::retryable(js_error(e)))?;
        let buffer = JsFuture::from(buffer)
            .await
            .map_err(|e| AttemptError::retryable(js_error(e)))?;
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        if let Some(limit) = limit
            && bytes.len() > limit
        {
            return Err(AttemptError::fatal(format!(
                "range returned more than {limit} bytes"
            )));
        }
        return Ok(bytes);
    };
    let Some(body) = response.body() else {
        return Ok(Vec::new());
    };
    let reader: web_sys::ReadableStreamDefaultReader = body
        .get_reader()
        .dyn_into()
        .map_err(|_| AttemptError::retryable("response body has no default reader"))?;
    let mut bytes = Vec::with_capacity(limit.unwrap_or(0));
    loop {
        let chunk = timeout(stall, JsFuture::from(reader.read()))
            .await
            .map_err(|_| AttemptError::retryable(format!("no data for {stall:?}")))?
            .map_err(|e| AttemptError::retryable(js_error(e)))?;
        let done = js_sys::Reflect::get(&chunk, &"done".into())
            .map(|done| done.is_truthy())
            .unwrap_or(true);
        if done {
            return Ok(bytes);
        }
        let value = js_sys::Reflect::get(&chunk, &"value".into())
            .map_err(|e| AttemptError::retryable(js_error(e)))?;
        let chunk = js_sys::Uint8Array::new(&value);
        if let Some(limit) = limit
            && bytes.len() + chunk.length() as usize > limit
        {
            let _ = reader.cancel();
            return Err(AttemptError::fatal(format!(
                "range returned more than {limit} bytes"
            )));
        }
        let start = bytes.len();
        bytes.resize(start + chunk.length() as usize, 0);
        chunk.copy_to(&mut bytes[start..]);
    }
}

pub(super) async fn fetch_http_attempt(
    client: &Client,
    url: &str,
    stall: Option<Duration>,
) -> Result<Vec<u8>, AttemptError> {
    let abort = AbortOnDrop(
        web_sys::AbortController::new().map_err(|e| AttemptError::retryable(js_error(e)))?,
    );
    let attempt = async {
        let response = super::within(stall, send(url, "GET", None, &abort.0))
            .await
            .ok_or_else(|| {
                AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()))
            })?
            .map_err(AttemptError::retryable)?;
        let status = response.status();
        if status == 404 {
            return Err(AttemptError {
                error: DataError::NotAvailable,
                retryable: false,
            });
        }
        if !(200..300).contains(&status) {
            return Err(AttemptError {
                error: DataError::Http(format!("request got {status}")),
                retryable: status >= 500,
            });
        }
        read_body(&response, stall, None).await
    };
    timeout(client.timeout, attempt)
        .await
        .map_err(|_| AttemptError::retryable(format!("timed out after {:?}", client.timeout)))?
}

pub(super) async fn fetch_http_range_attempt(
    client: &Client,
    url: &str,
    offset: u64,
    end: u64,
    wanted: usize,
    stall: Option<Duration>,
) -> Result<Vec<u8>, AttemptError> {
    let abort = AbortOnDrop(
        web_sys::AbortController::new().map_err(|e| AttemptError::retryable(js_error(e)))?,
    );
    let attempt = async {
        let response = super::within(stall, send(url, "GET", Some((offset, end)), &abort.0))
            .await
            .ok_or_else(|| {
                AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()))
            })?
            .map_err(AttemptError::retryable)?;
        let status = response.status();
        if status != 206 {
            return Err(AttemptError {
                error: DataError::Http(format!(
                    "range request got {status} (server ignores Range?)"
                )),
                retryable: status >= 500,
            });
        }
        let content_range = response
            .headers()
            .get("Content-Range")
            .ok()
            .flatten()
            .ok_or_else(|| {
                AttemptError::fatal("range response omitted or did not expose Content-Range")
            })?;
        super::validate_content_range(&content_range, offset, end).map_err(|error| {
            AttemptError {
                error,
                retryable: false,
            }
        })?;
        let bytes = read_body(&response, stall, Some(wanted)).await?;
        if bytes.len() != wanted {
            // A body that ends early is a dropped connection, not a bad server.
            return Err(AttemptError::retryable(format!(
                "range returned {} bytes, wanted {wanted}",
                bytes.len()
            )));
        }
        Ok(bytes)
    };
    timeout(client.timeout, attempt)
        .await
        .map_err(|_| AttemptError::retryable(format!("timed out after {:?}", client.timeout)))?
}

/// One attempt at a read of a hosted build, within the client's timeout.
pub(super) async fn fetch_archive_attempt(
    client: &Client,
    url: &str,
    offset: u64,
    end: u64,
    wanted: usize,
    stall: Option<Duration>,
    admission: &super::BlockAdmission,
) -> Result<Vec<u8>, AttemptError> {
    timeout(
        client.timeout,
        super::blocks::read(client, url, offset, end, wanted, stall, admission),
    )
    .await
    .map_err(|_| AttemptError::retryable(format!("timed out after {:?}", client.timeout)))?
}

/// One attempt at downloading block `index` of the hosted build at `url`.
pub(super) async fn fetch_block(
    _client: &Client,
    url: &str,
    index: u64,
    stall: Option<Duration>,
) -> Result<super::blocks::Block, AttemptError> {
    let size = crate::vectors::HOSTED_BUILD_BLOCK_BYTES;
    let abort = AbortOnDrop(
        web_sys::AbortController::new().map_err(|e| AttemptError::retryable(js_error(e)))?,
    );
    let target = crate::vectors::block_url(url, index);
    let response = super::within(stall, send(&target, "GET", None, &abort.0))
        .await
        .ok_or_else(|| {
            AttemptError::retryable(format!("no data for {:?}", stall.unwrap_or_default()))
        })?
        .map_err(AttemptError::retryable)?;
    let status = response.status();
    if status != 200 {
        return Err(AttemptError {
            error: DataError::Http(format!("archive block {index} got {status}")),
            retryable: status >= 500,
        });
    }
    // Every block but the archive's last is full; reads past the end of a
    // short block fail in `blocks::read`.
    let bytes = read_body(&response, stall, Some(size as usize)).await?;
    if bytes.is_empty() {
        return Err(AttemptError::retryable(format!(
            "archive block {index} was empty"
        )));
    }
    Ok(bytes.into())
}
