//! Blocking HTTP for browser workers, the counterpart of the native ureq
//! transport. Synchronous `XMLHttpRequest` is permitted in workers (never on
//! the page's main thread, where no Rust code runs), and must not be used on
//! the reactor worker, whose event loop resolves every async fetch.

use crate::DataError;
use std::time::Duration;
use wasm_bindgen::JsCast;

pub(crate) struct Response {
    /// 206 for a delivered byte range, however it was addressed.
    pub status: u16,
    pub body: Vec<u8>,
}

pub(crate) fn request(
    method: &str,
    url: &str,
    range: Option<(u64, u64)>,
    timeout: Duration,
) -> Result<Response, DataError> {
    match range {
        Some((offset, end)) if crate::vectors::is_block_archive(url) => {
            read_blocks(url, offset, end, timeout)
        }
        _ => send(method, url, range, timeout),
    }
}

/// Assembles bytes `offset..=end` of a hosted build from its block objects.
fn read_blocks(url: &str, offset: u64, end: u64, timeout: Duration) -> Result<Response, DataError> {
    let mut body = Vec::with_capacity((end - offset + 1) as usize);
    for (index, span) in crate::vectors::block_spans(offset, end) {
        let block = send("GET", &crate::vectors::block_url(url, index), None, timeout)?;
        if block.status != 200 {
            return Err(DataError::Http(format!(
                "{url} block {index}: got {}",
                block.status
            )));
        }
        let bytes = block.body.get(span).ok_or_else(|| {
            DataError::Http(format!(
                "range {offset}-{end} extends past the end of {url}"
            ))
        })?;
        body.extend_from_slice(bytes);
    }
    Ok(Response { status: 206, body })
}

fn send(
    method: &str,
    url: &str,
    range: Option<(u64, u64)>,
    timeout: Duration,
) -> Result<Response, DataError> {
    let http = |error: wasm_bindgen::JsValue| DataError::Http(format!("{url}: {error:?}"));
    let xhr = web_sys::XmlHttpRequest::new().map_err(http)?;
    xhr.open_with_async(method, url, false).map_err(http)?;
    xhr.set_timeout(timeout.as_millis().min(u128::from(u32::MAX)) as u32);
    xhr.set_response_type(web_sys::XmlHttpRequestResponseType::Arraybuffer);
    if let Some((offset, end)) = range {
        xhr.set_request_header("Range", &format!("bytes={offset}-{end}"))
            .map_err(http)?;
    }
    xhr.send().map_err(http)?;
    let status = xhr.status().map_err(http)?;
    if status == 0 {
        return Err(DataError::Http(format!("{url}: network error or timeout")));
    }
    let body = match xhr.response() {
        Ok(value) if !value.is_null() && !value.is_undefined() => {
            js_sys::Uint8Array::new(value.unchecked_ref::<js_sys::ArrayBuffer>()).to_vec()
        }
        _ => Vec::new(),
    };
    Ok(Response { status, body })
}
