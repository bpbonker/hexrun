//! Safe Rust wrapper around `libGenie` (Qualcomm Genie LLM runtime, ships
//! with QAIRT).
//!
//! Genie is the higher-level C API on top of QNN that handles
//! tokenization, sampling, KV cache, and prompt batching for LLMs. A
//! "dialog" is constructed from a JSON config that points at compiled
//! context-binary shards (`.bin`) and a tokenizer file (`tokenizer.json`).
//! After construction, `Dialog::query` runs an inference and returns the
//! generated text.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use qnn_sys as sys;
use thiserror::Error;
use tracing::{debug, info};

/// Errors raised by the Genie wrapper.
#[derive(Debug, Error)]
pub enum GenieError {
    /// `qnn-sys` was built without `QNN_SDK_ROOT`, so the real bindings are
    /// stub-only and the runtime will not function.
    #[error("Genie SDK was not present at build time (QNN_SDK_ROOT was unset)")]
    SdkMissing,
    /// Genie returned a non-success status code.
    #[error("Genie status {code} ({name})")]
    Status {
        /// The numeric status code.
        code: i32,
        /// A human-readable name for the status code.
        name: &'static str,
    },
    /// An argument to a Genie wrapper function was rejected before reaching
    /// the C API.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A Rust string contained an interior NUL byte and could not be
    /// converted to a C string.
    #[error("string contains interior NUL byte")]
    InteriorNul(#[from] std::ffi::NulError),
    /// I/O error reading a config file from disk.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path on which the I/O operation failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

fn status_name(code: i32) -> &'static str {
    match code {
        0 => "SUCCESS",
        1 => "WARNING_ABORTED",
        2 => "WARNING_BOUND_HANDLE",
        3 => "WARNING_PAUSED",
        4 => "WARNING_CONTEXT_EXCEEDED",
        -1 => "ERROR_GENERAL",
        -2 => "ERROR_INVALID_ARGUMENT",
        -3 => "ERROR_MEM_ALLOC",
        -4 => "ERROR_INVALID_CONFIG",
        -5 => "ERROR_INVALID_HANDLE",
        -6 => "ERROR_QUERY_FAILED",
        -7 => "ERROR_JSON_FORMAT",
        -8 => "ERROR_JSON_SCHEMA",
        -9 => "ERROR_JSON_VALUE",
        -10 => "ERROR_GENERATE_FAILED",
        -11 => "ERROR_GET_HANDLE_FAILED",
        -12 => "ERROR_APPLY_CONFIG_FAILED",
        -13 => "ERROR_SET_PARAMS_FAILED",
        -14 => "ERROR_BOUND_HANDLE",
        _ => "UNKNOWN",
    }
}

fn check(code: sys::Genie_Status_t) -> Result<(), GenieError> {
    // Genie's status codes split into three regimes:
    //   0          — SUCCESS
    //   positive   — WARNINGs (the call completed; partial result is valid):
    //                  1 = WARNING_ABORTED          (signal_abort interrupted)
    //                  2 = WARNING_BOUND_HANDLE
    //                  3 = WARNING_PAUSED
    //                  4 = WARNING_CONTEXT_EXCEEDED (max ctx; partial reply OK)
    //                Treating these as errors throws away a perfectly good
    //                partial response — chat clients then see a 500 instead
    //                of the truncated reply they should be able to render.
    //   negative   — hard ERRORs (the call failed; partial state is unsafe).
    if code == 0 {
        Ok(())
    } else if code > 0 {
        debug!(code, name = status_name(code), "Genie returned a warning; treating as success");
        Ok(())
    } else {
        Err(GenieError::Status {
            code,
            name: status_name(code),
        })
    }
}

/// Genie API version reported by the loaded `libGenie`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version.
    pub patch: u32,
}

/// Probe the loaded `libGenie` for its API version.
pub fn api_version() -> ApiVersion {
    // SAFETY: Genie_get*Version are pure functions with no preconditions.
    unsafe {
        ApiVersion {
            major: sys::Genie_getApiMajorVersion(),
            minor: sys::Genie_getApiMinorVersion(),
            patch: sys::Genie_getApiPatchVersion(),
        }
    }
}

/// Sentence code returned in streaming query callbacks. Mirrors
/// `GenieDialog_SentenceCode_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentenceCode {
    /// The chunk is the entire response.
    Complete,
    /// The chunk is the beginning of the response.
    Begin,
    /// The chunk is a continuation of the response.
    Continue,
    /// The chunk is the end of the response (or context limit was hit).
    End,
    /// The query was aborted.
    Abort,
    /// Genie rewound the KV cache to a prior point.
    Rewind,
    /// A previously paused query has resumed.
    Resume,
    /// Some other code Genie may emit in future SDK versions.
    Other(u32),
}

impl SentenceCode {
    fn from_raw(code: u32) -> Self {
        match code {
            0 => Self::Complete,
            1 => Self::Begin,
            2 => Self::Continue,
            3 => Self::End,
            4 => Self::Abort,
            5 => Self::Rewind,
            6 => Self::Resume,
            other => Self::Other(other),
        }
    }
}

fn sentence_code_to_raw(code: SentenceCode) -> sys::GenieDialog_SentenceCode_t {
    let raw: u32 = match code {
        SentenceCode::Complete => 0,
        SentenceCode::Begin => 1,
        SentenceCode::Continue => 2,
        SentenceCode::End => 3,
        SentenceCode::Abort => 4,
        SentenceCode::Rewind => 5,
        SentenceCode::Resume => 6,
        SentenceCode::Other(o) => o,
    };
    raw as sys::GenieDialog_SentenceCode_t
}

/// Internal RAII wrapper around `GenieDialogConfig_Handle_t`.
struct Config {
    handle: sys::GenieDialogConfig_Handle_t,
}

impl Config {
    fn from_json_str(json: &str) -> Result<Self, GenieError> {
        let cstr = CString::new(json)?;
        let mut handle: sys::GenieDialogConfig_Handle_t = std::ptr::null_mut();
        // SAFETY: cstr is a valid null-terminated string; handle is a valid
        // out-pointer; Genie owns the memory it allocates.
        unsafe {
            check(sys::GenieDialogConfig_createFromJson(
                cstr.as_ptr(),
                &mut handle,
            ))?;
        }
        debug!(?handle, "GenieDialogConfig created from JSON");
        Ok(Config { handle })
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle was returned by a successful create call and has
            // not been freed elsewhere.
            unsafe {
                let _ = sys::GenieDialogConfig_free(self.handle);
            }
        }
    }
}

/// A Genie dialog: a loaded LLM ready to answer queries.
///
/// Construct via [`Dialog::from_config_file`] (most common; reads the
/// `genie_config.json` from a npurun model bundle directory) or
/// [`Dialog::from_config_json`] (raw JSON string).
///
/// The dialog holds the entire compiled context-binary set in NPU shared
/// memory for the duration of its lifetime. Drop it to free the NPU
/// memory.
///
/// # Thread-safety
///
/// `Dialog` is marked `Send` + `Sync` so it can be held in an `Arc` and
/// moved between tokio tasks (needed for the HTTP server). Genie does
/// **not** support concurrent queries on a single dialog handle — callers
/// who share a `Dialog` across threads must serialize calls (e.g. with a
/// `Mutex` or `RwLock`). The handle itself is just a pointer to an opaque
/// C struct that is safe to pass between threads.
pub struct Dialog {
    handle: sys::GenieDialog_Handle_t,
    /// Held to ensure the config outlives the dialog (Genie may keep
    /// internal references).
    _config: Config,
}

// SAFETY: GenieDialog_Handle_t is an opaque pointer; Genie itself does not
// document thread-affinity requirements. Concurrent calls into a single
// dialog are explicitly *not* safe (Qualcomm's docs imply per-dialog
// serialization), so the higher-level type *Sync* is correct only when
// callers serialize via an external lock; that is the contract here.
unsafe impl Send for Dialog {}
unsafe impl Sync for Dialog {}

impl Dialog {
    /// Create a dialog from a JSON config string.
    ///
    /// The config typically references files (context-binary shards,
    /// tokenizer) by relative path. If those paths are relative, the
    /// process current working directory at the time of this call must be
    /// the directory that contains them. Prefer [`Self::from_config_file`]
    /// which handles this for you.
    pub fn from_config_json(json: &str) -> Result<Self, GenieError> {
        let config = Config::from_json_str(json)?;
        let mut handle: sys::GenieDialog_Handle_t = std::ptr::null_mut();
        // SAFETY: config.handle is a valid handle from a successful create
        // call; handle is a valid out-pointer.
        unsafe {
            check(sys::GenieDialog_create(config.handle, &mut handle))?;
        }
        let v = api_version();
        info!(
            handle = ?handle,
            "Genie dialog created (libGenie {}.{}.{})",
            v.major, v.minor, v.patch
        );
        Ok(Dialog {
            handle,
            _config: config,
        })
    }

    /// Create a dialog from a config file on disk.
    ///
    /// Reads the JSON, then creates the dialog with the file's parent
    /// directory as the current working directory so relative paths in the
    /// config (`ctx-bins`, `tokenizer.path`) resolve correctly. The CWD is
    /// restored before returning.
    pub fn from_config_file(path: &Path) -> Result<Self, GenieError> {
        let parent = path.parent().ok_or_else(|| {
            GenieError::InvalidArgument(format!(
                "config path {} has no parent directory",
                path.display()
            ))
        })?;
        let json = std::fs::read_to_string(path).map_err(|e| GenieError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        Self::from_config_json_in_dir(&json, parent)
    }

    /// Create a dialog from a JSON string while resolving relative paths
    /// against `parent`.
    ///
    /// Same chdir-and-restore dance as [`Self::from_config_file`], but
    /// for callers that have already loaded the JSON and (typically)
    /// patched it in memory before handing it to Genie.
    pub fn from_config_json_in_dir(json: &str, parent: &Path) -> Result<Self, GenieError> {
        let prev_cwd = std::env::current_dir().ok();
        std::env::set_current_dir(parent).map_err(|e| GenieError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
        debug!(cwd = %parent.display(), "chdir for Genie config relative paths");
        let result = Self::from_config_json(json);
        if let Some(prev) = prev_cwd {
            // Best-effort restore. If it fails the process is in an odd
            // state but the dialog was either created successfully or not.
            let _ = std::env::set_current_dir(prev);
        }
        result
    }

    /// Run a single, blocking query and return the full generated response.
    ///
    /// Genie streams response chunks to a callback; this method accumulates
    /// them into a `String`. For token-level streaming use
    /// [`Self::query_streaming`].
    pub fn query(&self, prompt: &str) -> Result<String, GenieError> {
        let mut buf = String::new();
        self.query_streaming(prompt, |chunk, _code| {
            buf.push_str(chunk);
        })?;
        Ok(buf)
    }

    /// Run a query and invoke `callback` for each response chunk.
    ///
    /// Equivalent to [`Self::query_streaming_with`] called with
    /// [`SentenceCode::Complete`] — Genie treats the prompt as a fresh,
    /// self-contained query and resets generation state for it.
    ///
    /// The callback is invoked synchronously from the C library, on the
    /// same thread that called this function. The function returns when
    /// Genie reports completion (or an error).
    pub fn query_streaming<F>(&self, prompt: &str, callback: F) -> Result<(), GenieError>
    where
        F: FnMut(&str, SentenceCode),
    {
        self.query_streaming_with(prompt, SentenceCode::Complete, callback)
    }

    /// Run a query with an explicit input [`SentenceCode`].
    ///
    /// The sentence code tells Genie how to interpret the prompt relative
    /// to any prior turns held in this dialog's KV cache:
    ///
    /// - [`SentenceCode::Complete`] — prompt is a self-contained query
    ///   (single-turn). Genie does not preserve nor look up KV state.
    /// - [`SentenceCode::Begin`] — prompt is the first sentence of a
    ///   multi-turn flow.
    /// - [`SentenceCode::Continue`] — prompt is a continuation of an
    ///   ongoing multi-turn flow.
    /// - [`SentenceCode::End`] — prompt closes a multi-turn flow.
    /// - [`SentenceCode::Rewind`] — prompt is a fresh transcript that
    ///   shares a prefix with what's already in the KV cache. Genie
    ///   matches the prefix, rewinds to the divergence point, and
    ///   re-prefills only the suffix. This is the multi-turn fast path:
    ///   send the full transcript every turn and pay the prefill cost
    ///   only on the new tokens.
    pub fn query_streaming_with<F>(
        &self,
        prompt: &str,
        code: SentenceCode,
        mut callback: F,
    ) -> Result<(), GenieError>
    where
        F: FnMut(&str, SentenceCode),
    {
        let cstr = CString::new(prompt)?;

        // We pass a pointer to `callback` (as a trait object) through Genie's
        // `userData` channel. Genie's QueryCallback_t fires synchronously
        // from the same thread, so we don't need Send/Sync here.
        let mut user_data = UserData {
            callback: &mut callback,
        };

        let raw_code = sentence_code_to_raw(code);

        // SAFETY: cstr is a valid null-terminated string for the duration of
        // the call; user_data lives on the stack until after the C call
        // returns; the callback function pointer is `extern "C"`.
        let rc = unsafe {
            sys::GenieDialog_query(
                self.handle,
                cstr.as_ptr(),
                raw_code,
                Some(query_callback::<F>),
                &mut user_data as *mut UserData<F> as *const c_void,
            )
        };
        check(rc)
    }

    /// Reset the dialog's internal KV cache, returning it to the same state
    /// as a freshly-created dialog from the same config.
    pub fn reset(&self) -> Result<(), GenieError> {
        // SAFETY: handle is a valid handle from a successful create call.
        let rc = unsafe { sys::GenieDialog_reset(self.handle) };
        check(rc)
    }

    /// Signal the dialog to abort an in-flight `query_streaming*` call.
    ///
    /// Designed by Qualcomm to be called from a different thread than the
    /// one blocked inside Genie; the next time Genie checks for signals
    /// (between tokens) it returns with `SentenceCode::Abort`. The
    /// callback is invoked one final time with the abort code, then
    /// `query_streaming` returns with success status. The dialog handle
    /// remains usable; call [`Self::reset`] to clear KV cache state if
    /// the next query should not see the aborted prompt.
    ///
    /// Returns `Ok(())` if the signal was delivered (whether or not it
    /// actually interrupted anything — calling on an idle dialog is a
    /// no-op from Genie's perspective). Returns an error if the dialog
    /// handle is invalid or Genie rejects the action.
    pub fn signal_abort(&self) -> Result<(), GenieError> {
        // SAFETY: handle is a valid handle from a successful create call.
        // ACTION_ABORT = 1 from Genie's enum; the bindgen constant lives
        // at sys::GenieDialog_Action_t_GENIE_DIALOG_ACTION_ABORT.
        let rc = unsafe {
            sys::GenieDialog_signal(
                self.handle,
                sys::GenieDialog_Action_t_GENIE_DIALOG_ACTION_ABORT,
            )
        };
        check(rc)
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle was returned by a successful create call and
            // has not been freed elsewhere.
            unsafe {
                let _ = sys::GenieDialog_free(self.handle);
            }
        }
    }
}

/// User data passed through Genie's query callback bridge.
struct UserData<'a, F: FnMut(&str, SentenceCode)> {
    callback: &'a mut F,
}

/// Genie query callback bridge, parameterized by the user's closure type.
extern "C" fn query_callback<F: FnMut(&str, SentenceCode)>(
    response: *const c_char,
    sentence: sys::GenieDialog_SentenceCode_t,
    user_data: *const c_void,
) {
    if response.is_null() || user_data.is_null() {
        return;
    }
    // SAFETY: `response` is a valid C string per the callback contract;
    // `user_data` is the `UserData<F>` we passed in.
    let chunk = unsafe { CStr::from_ptr(response) };
    let user_data = unsafe { &mut *(user_data as *mut UserData<F>) };
    let code = SentenceCode::from_raw(sentence as u32);
    let chunk = chunk.to_string_lossy();
    (user_data.callback)(&chunk, code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_is_at_least_1_17() {
        let v = api_version();
        // Genie has been at major=1 since QAIRT 2.x; expect >=1.17 with
        // QAIRT 2.45. Test guards against accidental ABI breakage.
        assert!(v.major >= 1, "unexpected major: {}", v.major);
        assert!(
            (v.major, v.minor) >= (1, 17),
            "unexpected version: {}.{}.{}",
            v.major,
            v.minor,
            v.patch
        );
    }

    #[test]
    fn config_from_invalid_json_errors() {
        // Empty JSON object isn't a valid Genie config -- expect a structured
        // status error, not a panic.
        match Config::from_json_str("{}") {
            Err(GenieError::Status { .. }) => {}
            Err(other) => panic!("expected GenieError::Status, got {other:?}"),
            Ok(_) => panic!("expected an error from empty config"),
        }
    }

    #[test]
    fn dialog_from_invalid_json_errors() {
        match Dialog::from_config_json("not json") {
            Err(GenieError::Status { .. }) => {}
            Err(other) => panic!("expected GenieError::Status, got {other:?}"),
            Ok(_) => panic!("expected an error from invalid JSON"),
        }
    }

    #[test]
    fn sentence_code_round_trip() {
        for (raw, expected) in [
            (0, SentenceCode::Complete),
            (1, SentenceCode::Begin),
            (2, SentenceCode::Continue),
            (3, SentenceCode::End),
            (4, SentenceCode::Abort),
            (5, SentenceCode::Rewind),
            (6, SentenceCode::Resume),
        ] {
            assert_eq!(SentenceCode::from_raw(raw), expected);
        }
        assert_eq!(SentenceCode::from_raw(99), SentenceCode::Other(99));
    }
}
