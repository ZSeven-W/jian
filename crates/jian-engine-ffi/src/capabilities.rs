use crate::error::{read_utf8, FfiError, FfiResult, STRING_CAP};
use crate::lifecycle::{call_engine, Lifecycle};
use crate::{JianEngine, JianStatus};
use async_trait::async_trait;
use futures::channel::oneshot;
use jian_core::action::services::{
    AsyncFeedback, ClipboardService, HttpRequest, HttpResponse, NetworkClient, PlatformService,
    ServiceError,
};
use jian_core::render::image_store::{ImageAdmission, ImageResolver};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::c_void;
use std::future::Future;
use std::mem::size_of;
use std::pin::Pin;
use std::ptr;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll};

pub(crate) const CAPABILITY_CAP: usize = 64 * 1024 * 1024;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JianCapabilityKind {
    HttpFetch = 0,
    Confirm = 1,
    ClipboardRead = 2,
    ClipboardWrite = 3,
    ImageFetch = 4,
    OpenUrl = 5,
}

impl JianCapabilityKind {
    fn from_i32(value: i32) -> Option<Self> {
        Some(match value {
            0 => Self::HttpFetch,
            1 => Self::Confirm,
            2 => Self::ClipboardRead,
            3 => Self::ClipboardWrite,
            4 => Self::ImageFetch,
            5 => Self::OpenUrl,
            _ => return None,
        })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianHeader {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianHttpFetchRequest {
    pub method_ptr: *const u8,
    pub method_len: usize,
    pub url_ptr: *const u8,
    pub url_len: usize,
    pub headers: *const JianHeader,
    pub headers_len: usize,
    pub has_body: bool,
    pub body_ptr: *const u8,
    pub body_len: usize,
    pub has_timeout: bool,
    pub timeout_ms: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianConfirmRequest {
    pub title_ptr: *const u8,
    pub title_len: usize,
    pub message_ptr: *const u8,
    pub message_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianClipboardReadRequest {
    pub reserved: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianClipboardWriteRequest {
    pub text_ptr: *const u8,
    pub text_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianImageFetchRequest {
    pub url_ptr: *const u8,
    pub url_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianOpenUrlRequest {
    pub url_ptr: *const u8,
    pub url_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union JianCapabilityRequestData {
    pub http_fetch: JianHttpFetchRequest,
    pub confirm: JianConfirmRequest,
    pub clipboard_read: JianClipboardReadRequest,
    pub clipboard_write: JianClipboardWriteRequest,
    pub image_fetch: JianImageFetchRequest,
    pub open_url: JianOpenUrlRequest,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianCapabilityRequest {
    pub size: usize,
    pub kind: JianCapabilityKind,
    pub data: JianCapabilityRequestData,
}

pub type JianCapabilityRequestCallback = unsafe extern "C" fn(
    user_data: *mut c_void,
    request_id: u64,
    request: *const JianCapabilityRequest,
);
pub type JianCapabilityCancelled = unsafe extern "C" fn(user_data: *mut c_void, request_id: u64);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianHttpFetchResult {
    pub ok: bool,
    pub status: u16,
    pub headers: *const JianHeader,
    pub headers_len: usize,
    pub body_ptr: *const u8,
    pub body_len: usize,
    pub error_ptr: *const u8,
    pub error_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianConfirmResult {
    pub value: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianClipboardReadResult {
    pub ok: bool,
    pub text_ptr: *const u8,
    pub text_len: usize,
    pub error_ptr: *const u8,
    pub error_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianClipboardWriteResult {
    pub ok: bool,
    pub error_ptr: *const u8,
    pub error_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianImageFetchResult {
    pub ok: bool,
    pub bytes_ptr: *const u8,
    pub bytes_len: usize,
    pub error_ptr: *const u8,
    pub error_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianOpenUrlResult {
    pub ok: bool,
    pub error_ptr: *const u8,
    pub error_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union JianCapabilityResultData {
    pub http_fetch: JianHttpFetchResult,
    pub confirm: JianConfirmResult,
    pub clipboard_read: JianClipboardReadResult,
    pub clipboard_write: JianClipboardWriteResult,
    pub image_fetch: JianImageFetchResult,
    pub open_url: JianOpenUrlResult,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JianCapabilityResult {
    pub size: usize,
    pub kind: i32,
    pub data: JianCapabilityResultData,
}

#[derive(Debug)]
enum OwnedRequest {
    Http {
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Option<Vec<u8>>,
        timeout_ms: Option<u64>,
    },
    Confirm {
        title: String,
        message: String,
    },
    ClipboardRead,
    ClipboardWrite {
        text: String,
    },
    ImageFetch {
        url: String,
    },
    OpenUrl {
        url: String,
    },
}

impl OwnedRequest {
    fn kind(&self) -> JianCapabilityKind {
        match self {
            Self::Http { .. } => JianCapabilityKind::HttpFetch,
            Self::Confirm { .. } => JianCapabilityKind::Confirm,
            Self::ClipboardRead => JianCapabilityKind::ClipboardRead,
            Self::ClipboardWrite { .. } => JianCapabilityKind::ClipboardWrite,
            Self::ImageFetch { .. } => JianCapabilityKind::ImageFetch,
            Self::OpenUrl { .. } => JianCapabilityKind::OpenUrl,
        }
    }

    fn payload_bytes(&self) -> Option<usize> {
        let values: Vec<usize> = match self {
            Self::Http {
                method,
                url,
                headers,
                body,
                ..
            } => std::iter::once(method.len())
                .chain(std::iter::once(url.len()))
                .chain(
                    headers
                        .iter()
                        .flat_map(|(name, value)| [name.len(), value.len()]),
                )
                .chain(body.iter().map(Vec::len))
                .collect(),
            Self::Confirm { title, message } => vec![title.len(), message.len()],
            Self::ClipboardRead => Vec::new(),
            Self::ClipboardWrite { text } => vec![text.len()],
            Self::ImageFetch { url } | Self::OpenUrl { url } => vec![url.len()],
        };
        values
            .into_iter()
            .try_fold(0usize, |total, length| total.checked_add(length))
    }
}

enum OwnedResponse {
    Http(Result<HttpResponse, String>),
    Confirm(bool),
    ClipboardRead(Result<String, String>),
    ClipboardWrite(Result<(), String>),
    Image(Result<Vec<u8>, String>),
    OpenUrl(Result<(), String>),
}

struct Pending {
    kind: JianCapabilityKind,
    sender: Option<oneshot::Sender<OwnedResponse>>,
}

#[derive(Default)]
struct BrokerState {
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    outgoing: VecDeque<(u64, OwnedRequest)>,
    cancelled: VecDeque<u64>,
    warnings: Vec<String>,
}

pub(crate) struct CapabilityBridge {
    state: Rc<RefCell<BrokerState>>,
    enabled: bool,
}

impl CapabilityBridge {
    pub(crate) fn new(enabled: bool) -> Rc<Self> {
        Rc::new(Self {
            state: Rc::new(RefCell::new(BrokerState::default())),
            enabled,
        })
    }

    fn enqueue(&self, request: OwnedRequest) -> Result<PendingWait, String> {
        if !self.enabled {
            return Err("capability_request callback is unavailable".into());
        }
        if request
            .payload_bytes()
            .map_or(true, |size| size > CAPABILITY_CAP)
        {
            return Err("capability request payload exceeds 64 MiB".into());
        }
        let (sender, receiver) = oneshot::channel();
        let mut state = self.state.borrow_mut();
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let id = state.next_id;
        state.pending.insert(
            id,
            Pending {
                kind: request.kind(),
                sender: Some(sender),
            },
        );
        state.outgoing.push_back((id, request));
        Ok(PendingWait {
            id,
            receiver,
            state: Rc::downgrade(&self.state),
        })
    }

    fn enqueue_detached(&self, request: OwnedRequest) -> Result<(), String> {
        if !self.enabled {
            return Err("capability_request callback is unavailable".into());
        }
        if request
            .payload_bytes()
            .map_or(true, |size| size > CAPABILITY_CAP)
        {
            return Err("capability request payload exceeds 64 MiB".into());
        }
        let mut state = self.state.borrow_mut();
        state.next_id = state.next_id.wrapping_add(1).max(1);
        let id = state.next_id;
        state.pending.insert(
            id,
            Pending {
                kind: request.kind(),
                sender: None,
            },
        );
        state.outgoing.push_back((id, request));
        Ok(())
    }

    pub(crate) fn emit_callbacks(&self, lifecycle: &mut Lifecycle) {
        let outgoing: Vec<_> = self.state.borrow_mut().outgoing.drain(..).collect();
        if let Some(callback) = lifecycle.callbacks.capability_request {
            for (id, request) in outgoing {
                unsafe { emit_request(callback, lifecycle.callbacks.user_data, id, &request) };
            }
        }
        let cancelled: Vec<_> = self.state.borrow_mut().cancelled.drain(..).collect();
        if let Some(callback) = lifecycle.callbacks.capability_cancelled {
            for id in cancelled {
                unsafe { callback(lifecycle.callbacks.user_data, id) };
            }
        }
        let warnings: Vec<_> = self.state.borrow_mut().warnings.drain(..).collect();
        for warning in warnings {
            lifecycle.runtime.push_load_warning(warning);
        }
    }

    pub(crate) fn cancel_all(&self) {
        let mut state = self.state.borrow_mut();
        let ids: Vec<_> = state.pending.keys().copied().collect();
        state.pending.clear();
        state.outgoing.clear();
        state.cancelled.extend(ids);
    }

    fn complete(
        &self,
        id: u64,
        kind: JianCapabilityKind,
        response: OwnedResponse,
    ) -> FfiResult<()> {
        let mut state = self.state.borrow_mut();
        let Some(pending) = state.pending.remove(&id) else {
            return Err(FfiError::invalid("capability request id is not active"));
        };
        if pending.kind != kind {
            state.pending.insert(id, pending);
            return Err(FfiError::invalid(
                "capability result kind does not match request",
            ));
        }
        if let Some(sender) = pending.sender {
            let _ = sender.send(response);
        } else if let OwnedResponse::OpenUrl(Err(error)) = response {
            state.warnings.push(format!("open_url: {error}"));
        }
        Ok(())
    }
}

struct PendingWait {
    id: u64,
    receiver: oneshot::Receiver<OwnedResponse>,
    state: Weak<RefCell<BrokerState>>,
}

impl Future for PendingWait {
    type Output = Result<OwnedResponse, String>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.receiver)
            .poll(context)
            .map(|value| value.map_err(|_| "capability request was cancelled".into()))
    }
}

impl Drop for PendingWait {
    fn drop(&mut self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let mut state = state.borrow_mut();
        if state.pending.remove(&self.id).is_some() {
            state.outgoing.retain(|(id, _)| *id != self.id);
            state.cancelled.push_back(self.id);
        }
    }
}

#[async_trait(?Send)]
impl NetworkClient for CapabilityBridge {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, String> {
        let mut headers: Vec<_> = request.headers.into_iter().collect();
        let body = request
            .body
            .map(|value| serde_json::to_vec(&value).map_err(|error| error.to_string()))
            .transpose()?;
        if body.is_some()
            && !headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        match self
            .enqueue(OwnedRequest::Http {
                method: request.method,
                url: request.url,
                headers,
                body,
                timeout_ms: request.timeout_ms,
            })?
            .await?
        {
            OwnedResponse::Http(result) => result,
            _ => Err("capability response kind mismatch".into()),
        }
    }
}

#[async_trait(?Send)]
impl AsyncFeedback for CapabilityBridge {
    async fn confirm(&self, title: &str, message: &str) -> bool {
        let Ok(wait) = self.enqueue(OwnedRequest::Confirm {
            title: title.into(),
            message: message.into(),
        }) else {
            return false;
        };
        match wait.await {
            Ok(OwnedResponse::Confirm(value)) => value,
            _ => false,
        }
    }
}

#[async_trait(?Send)]
impl ClipboardService for CapabilityBridge {
    async fn read_text(&self) -> Result<String, ServiceError> {
        let wait = self
            .enqueue(OwnedRequest::ClipboardRead)
            .map_err(ServiceError)?;
        match wait.await.map_err(ServiceError)? {
            OwnedResponse::ClipboardRead(result) => result.map_err(ServiceError),
            _ => Err(ServiceError("capability response kind mismatch".into())),
        }
    }

    async fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        let wait = self
            .enqueue(OwnedRequest::ClipboardWrite { text: text.into() })
            .map_err(ServiceError)?;
        match wait.await.map_err(ServiceError)? {
            OwnedResponse::ClipboardWrite(result) => result.map_err(ServiceError),
            _ => Err(ServiceError("capability response kind mismatch".into())),
        }
    }
}

impl PlatformService for CapabilityBridge {
    fn open_url(&self, url: &str) -> Result<(), ServiceError> {
        self.enqueue_detached(OwnedRequest::OpenUrl { url: url.into() })
            .map_err(ServiceError)
    }
}

#[async_trait(?Send)]
impl ImageResolver for CapabilityBridge {
    fn admission(&self, source: &str) -> Result<Option<ImageAdmission>, String> {
        if source.starts_with("http://") || source.starts_with("https://") {
            Ok(Some(ImageAdmission {
                key: source.into(),
                request_source: source.into(),
                requires_network: true,
            }))
        } else {
            Ok(None)
        }
    }

    async fn resolve(&self, url: &str) -> Result<Vec<u8>, String> {
        match self
            .enqueue(OwnedRequest::ImageFetch { url: url.into() })?
            .await?
        {
            OwnedResponse::Image(result) => result,
            _ => Err("capability response kind mismatch".into()),
        }
    }
}

unsafe fn emit_request(
    callback: JianCapabilityRequestCallback,
    user_data: *mut c_void,
    id: u64,
    owned: &OwnedRequest,
) {
    let mut abi_headers = Vec::new();
    let data = match owned {
        OwnedRequest::Http {
            method,
            url,
            headers,
            body,
            timeout_ms,
        } => {
            abi_headers.extend(headers.iter().map(|(name, value)| JianHeader {
                name_ptr: name.as_ptr(),
                name_len: name.len(),
                value_ptr: value.as_ptr(),
                value_len: value.len(),
            }));
            JianCapabilityRequestData {
                http_fetch: JianHttpFetchRequest {
                    method_ptr: method.as_ptr(),
                    method_len: method.len(),
                    url_ptr: url.as_ptr(),
                    url_len: url.len(),
                    headers: abi_headers.as_ptr(),
                    headers_len: abi_headers.len(),
                    has_body: body.is_some(),
                    body_ptr: body.as_ref().map_or(ptr::null(), |bytes| bytes.as_ptr()),
                    body_len: body.as_ref().map_or(0, Vec::len),
                    has_timeout: timeout_ms.is_some(),
                    timeout_ms: timeout_ms.unwrap_or(0),
                },
            }
        }
        OwnedRequest::Confirm { title, message } => JianCapabilityRequestData {
            confirm: JianConfirmRequest {
                title_ptr: title.as_ptr(),
                title_len: title.len(),
                message_ptr: message.as_ptr(),
                message_len: message.len(),
            },
        },
        OwnedRequest::ClipboardRead => JianCapabilityRequestData {
            clipboard_read: JianClipboardReadRequest { reserved: 0 },
        },
        OwnedRequest::ClipboardWrite { text } => JianCapabilityRequestData {
            clipboard_write: JianClipboardWriteRequest {
                text_ptr: text.as_ptr(),
                text_len: text.len(),
            },
        },
        OwnedRequest::ImageFetch { url } => JianCapabilityRequestData {
            image_fetch: JianImageFetchRequest {
                url_ptr: url.as_ptr(),
                url_len: url.len(),
            },
        },
        OwnedRequest::OpenUrl { url } => JianCapabilityRequestData {
            open_url: JianOpenUrlRequest {
                url_ptr: url.as_ptr(),
                url_len: url.len(),
            },
        },
    };
    let request = JianCapabilityRequest {
        size: size_of::<JianCapabilityRequest>(),
        kind: owned.kind(),
        data,
    };
    unsafe { callback(user_data, id, &request) };
}

unsafe fn read_headers(
    pointer: *const JianHeader,
    length: usize,
) -> FfiResult<BTreeMap<String, String>> {
    if length == 0 {
        return Ok(BTreeMap::new());
    }
    if pointer.is_null() || length > isize::MAX as usize / size_of::<JianHeader>() {
        return Err(FfiError::invalid(
            "capability headers pointer or length is invalid",
        ));
    }
    let headers = unsafe { std::slice::from_raw_parts(pointer, length) };
    let mut output = BTreeMap::new();
    let mut total = 0usize;
    for header in headers {
        total = total
            .checked_add(header.name_len)
            .and_then(|value| value.checked_add(header.value_len))
            .ok_or_else(|| FfiError::invalid("capability header bytes overflow"))?;
        if total > CAPABILITY_CAP {
            return Err(FfiError::invalid("capability headers exceed 64 MiB"));
        }
        let name =
            unsafe { read_utf8(header.name_ptr, header.name_len, STRING_CAP, "header name") }?;
        let value = unsafe {
            read_utf8(
                header.value_ptr,
                header.value_len,
                STRING_CAP,
                "header value",
            )
        }?;
        output.insert(name, value);
    }
    Ok(output)
}

unsafe fn read_bytes(pointer: *const u8, length: usize, label: &str) -> FfiResult<Vec<u8>> {
    if length > CAPABILITY_CAP {
        return Err(FfiError::invalid(format!("{label} exceeds 64 MiB")));
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null() || length > isize::MAX as usize {
        return Err(FfiError::invalid(format!(
            "{label} pointer or length is invalid"
        )));
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec())
}

unsafe fn parse_result(
    kind: JianCapabilityKind,
    data: JianCapabilityResultData,
) -> FfiResult<OwnedResponse> {
    Ok(match kind {
        JianCapabilityKind::HttpFetch => {
            let value = unsafe { data.http_fetch };
            if value.ok {
                let headers = unsafe { read_headers(value.headers, value.headers_len) }?;
                let bytes =
                    unsafe { read_bytes(value.body_ptr, value.body_len, "HTTP response body") }?;
                let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                    Value::String(String::from_utf8_lossy(&bytes).into_owned())
                });
                OwnedResponse::Http(Ok(HttpResponse {
                    status: value.status,
                    headers,
                    body,
                }))
            } else {
                OwnedResponse::Http(Err(unsafe {
                    read_utf8(value.error_ptr, value.error_len, STRING_CAP, "HTTP error")
                }?))
            }
        }
        JianCapabilityKind::Confirm => OwnedResponse::Confirm(unsafe { data.confirm }.value),
        JianCapabilityKind::ClipboardRead => {
            let value = unsafe { data.clipboard_read };
            let result = if value.ok {
                Ok(unsafe {
                    read_utf8(value.text_ptr, value.text_len, STRING_CAP, "clipboard text")
                }?)
            } else {
                Err(unsafe {
                    read_utf8(
                        value.error_ptr,
                        value.error_len,
                        STRING_CAP,
                        "clipboard error",
                    )
                }?)
            };
            OwnedResponse::ClipboardRead(result)
        }
        JianCapabilityKind::ClipboardWrite => {
            let value = unsafe { data.clipboard_write };
            OwnedResponse::ClipboardWrite(if value.ok {
                Ok(())
            } else {
                Err(unsafe {
                    read_utf8(
                        value.error_ptr,
                        value.error_len,
                        STRING_CAP,
                        "clipboard error",
                    )
                }?)
            })
        }
        JianCapabilityKind::ImageFetch => {
            let value = unsafe { data.image_fetch };
            let result = if value.ok {
                match unsafe { read_bytes(value.bytes_ptr, value.bytes_len, "image response") } {
                    Ok(bytes) => Ok(bytes),
                    Err(error) => Err(error.message),
                }
            } else {
                Err(unsafe {
                    read_utf8(value.error_ptr, value.error_len, STRING_CAP, "image error")
                }?)
            };
            OwnedResponse::Image(result)
        }
        JianCapabilityKind::OpenUrl => {
            let value = unsafe { data.open_url };
            OwnedResponse::OpenUrl(if value.ok {
                Ok(())
            } else {
                Err(unsafe {
                    read_utf8(
                        value.error_ptr,
                        value.error_len,
                        STRING_CAP,
                        "open URL error",
                    )
                }?)
            })
        }
    })
}

/// Complete an outstanding platform capability request.
///
/// # Safety
/// `engine` must be live and `result` must expose a complete result struct.
#[no_mangle]
pub unsafe extern "C" fn jian_capability_result(
    engine: *mut JianEngine,
    request_id: u64,
    result: *const JianCapabilityResult,
) -> JianStatus {
    unsafe {
        call_engine(engine, |lifecycle| {
            if request_id == 0 || result.is_null() {
                return Err(FfiError::invalid(
                    "capability result pointer or id is invalid",
                ));
            }
            if (*result).size != size_of::<JianCapabilityResult>() {
                return Err(FfiError::invalid("capability result size is invalid"));
            }
            let kind = JianCapabilityKind::from_i32((*result).kind)
                .ok_or_else(|| FfiError::invalid("capability result kind is invalid"))?;
            let response = parse_result(kind, (*result).data)?;
            lifecycle.capabilities.complete(request_id, kind, response)
        })
    }
}
