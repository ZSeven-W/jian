use super::{await_with_wake, AbortRegistry, BrowserCleanup};
use jian_core::action::services::{HttpRequest, HttpResponse, NetworkClient};
use js_sys::Uint8Array;
use std::collections::BTreeMap;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    ReadableStreamDefaultReader, ReadableStreamReadResult, Request, RequestCredentials,
    RequestInit, RequestRedirect, Response, Window,
};

const MAX_IMAGE_TRANSFER_BYTES: usize = 64 * 1024 * 1024;

struct TimeoutLease {
    window: Option<Window>,
    id: i32,
    callback: Option<Closure<dyn FnMut()>>,
    cleanup: BrowserCleanup,
}

impl TimeoutLease {
    fn arm(
        controller: web_sys::AbortController,
        timeout_ms: u64,
        cleanup: BrowserCleanup,
    ) -> Option<Self> {
        let window = web_sys::window()?;
        let callback = Closure::once(move || controller.abort());
        let id = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                timeout_ms.min(i32::MAX as u64) as i32,
            )
            .ok()?;
        Some(Self {
            window: Some(window),
            id,
            callback: Some(callback),
            cleanup,
        })
    }
}

impl Drop for TimeoutLease {
    fn drop(&mut self) {
        let Some(window) = self.window.take() else {
            return;
        };
        let callback = self.callback.take();
        let id = self.id;
        self.cleanup.run(move || {
            window.clear_timeout_with_handle(id);
            drop(callback);
        });
    }
}

pub struct WebNetwork {
    aborts: AbortRegistry,
    wake: Rc<dyn Fn()>,
}

impl WebNetwork {
    pub fn new(aborts: AbortRegistry, wake: Rc<dyn Fn()>) -> Self {
        Self { aborts, wake }
    }
}

#[async_trait::async_trait(?Send)]
impl NetworkClient for WebNetwork {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let lease = self.aborts.lease().map_err(js_error)?;
        let init = RequestInit::new();
        init.set_method(&request.method);
        init.set_credentials(RequestCredentials::Omit);
        init.set_redirect(RequestRedirect::Error);
        init.set_signal(Some(&lease.signal()));
        if let Some(body) = request.body {
            let text = serde_json::to_string(&body).map_err(|error| error.to_string())?;
            init.set_body(&JsValue::from_str(&text));
        }
        let web_request = Request::new_with_str_and_init(&request.url, &init).map_err(js_error)?;
        for (name, value) in request.headers {
            web_request.headers().set(&name, &value).map_err(js_error)?;
        }
        // Declared after the abort lease so cancellation drops the timeout
        // first, clearing the browser timer while its Closure is still live.
        let timeout = request.timeout_ms.and_then(|timeout| {
            TimeoutLease::arm(lease.controller(), timeout, self.aborts.cleanup())
        });
        let result = fetch_response(web_request, &self.wake).await;
        drop(timeout);
        lease.complete();
        (self.wake)();
        result
    }

    async fn connect_websocket(
        &self,
        _url: String,
    ) -> Result<Rc<dyn jian_core::action::services::WebSocketSession>, String> {
        (self.wake)();
        Err("websocket unsupported on this host".into())
    }
}

async fn fetch_response(request: Request, wake: &Rc<dyn Fn()>) -> Result<HttpResponse, String> {
    let window = web_sys::window().ok_or("window unavailable")?;
    let response: Response = await_with_wake(window.fetch_with_request(&request), wake)
        .await
        .map_err(js_error)?
        .dyn_into()
        .map_err(|_| "fetch returned a non-Response value")?;
    let status = response.status();
    let text = await_with_wake(response.text().map_err(js_error)?, wake)
        .await
        .map_err(js_error)?
        .as_string()
        .unwrap_or_default();
    let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
    Ok(HttpResponse {
        status,
        headers: BTreeMap::new(),
        body,
    })
}

pub async fn fetch_bytes(
    url: &str,
    aborts: &AbortRegistry,
    wake: &Rc<dyn Fn()>,
) -> Result<Vec<u8>, String> {
    fetch_bytes_with_limit(url, aborts, wake, MAX_IMAGE_TRANSFER_BYTES).await
}

pub(crate) async fn fetch_bytes_with_limit(
    url: &str,
    aborts: &AbortRegistry,
    wake: &Rc<dyn Fn()>,
    limit: usize,
) -> Result<Vec<u8>, String> {
    let lease = aborts.lease().map_err(js_error)?;
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_credentials(RequestCredentials::Omit);
    init.set_redirect(RequestRedirect::Error);
    init.set_signal(Some(&lease.signal()));
    let request = Request::new_with_str_and_init(url, &init).map_err(js_error)?;
    let result = async {
        let response: Response = await_with_wake(
            web_sys::window()
                .ok_or("window unavailable")?
                .fetch_with_request(&request),
            wake,
        )
        .await
        .map_err(js_error)?
        .dyn_into()
        .map_err(|_| "fetch returned a non-Response value")?;
        if !response.ok() {
            return Err(format!("image fetch returned HTTP {}", response.status()));
        }
        read_response_bytes(&response, limit, wake).await
    }
    .await;
    if result.is_ok() {
        lease.complete();
    } else {
        // Dropping the active lease aborts the actual browser signal before
        // the failure escapes, including over-cap responses.
        drop(lease);
    }
    wake();
    result
}

async fn read_response_bytes(
    response: &Response,
    limit: usize,
    wake: &Rc<dyn Fn()>,
) -> Result<Vec<u8>, String> {
    if let Some(value) = response.headers().get("Content-Length").map_err(js_error)? {
        if let Ok(length) = value.parse::<u64>() {
            if length > limit as u64 {
                return Err(format!("image response exceeds {} bytes", limit));
            }
        }
    }
    let stream = response.body().ok_or("image response body unavailable")?;
    let reader = ReadableStreamDefaultReader::new(&stream).map_err(js_error)?;
    let mut bytes = Vec::new();
    loop {
        // `read()` is specified to fulfill with the
        // ReadableStreamReadResult dictionary. It is not a constructible JS
        // class, so an instanceof-style checked cast can reject a perfectly
        // valid browser result in Chrome. Treat the specified dictionary
        // structurally and inspect its fields through the generated accessors.
        let result: ReadableStreamReadResult = await_with_wake(reader.read(), wake)
            .await
            .map_err(js_error)?
            .unchecked_into();
        if result.get_done().unwrap_or(false) {
            break;
        }
        let chunk = Uint8Array::new(&result.get_value());
        let chunk_len = usize::try_from(chunk.length()).map_err(|_| "image chunk too large")?;
        let prospective = bytes
            .len()
            .checked_add(chunk_len)
            .ok_or("image response size overflow")?;
        if prospective > limit {
            return Err(format!("image response exceeds {} bytes", limit));
        }
        bytes
            .try_reserve(chunk_len)
            .map_err(|_| "image response allocation failed")?;
        let old_len = bytes.len();
        bytes.resize(prospective, 0);
        chunk.copy_to(&mut bytes[old_len..]);
    }
    reader.release_lock();
    Ok(bytes)
}

fn js_error(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| format!("browser request failed: {error:?}"))
}
