use libloading::{Library, Symbol};
use litebubbles_validation_provider::{
    HardwareConfig, InitialPayload, MAX_JSON_LINE_BYTES, MAX_VALIDATION_BUFFER_BYTES,
    REFERENCE_ADDRESS, REFERENCE_SYMBOL, ResultPayload, SessionInfoPayload,
    VALIDATION_CTX_KEY_ESTABLISHMENT_ADDRESS, VALIDATION_CTX_NEW_ADDRESS,
    VALIDATION_CTX_SIGN_ADDRESS, ValidatedComponent,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::env;
use std::ffi::c_void;
use std::io::{self, BufRead, BufReader, Write};
use std::mem::transmute;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
enum HelperError {
    #[error("invalid helper arguments")]
    Arguments,
    #[error("unsupported helper platform")]
    Platform,
    #[error("validation component is unavailable")]
    Component,
    #[error("validation component ABI is unavailable")]
    Abi,
    #[error("validation operation failed")]
    Operation,
    #[error("validation input or output was malformed")]
    Malformed,
    #[error("validation input or output exceeded a limit")]
    Oversized,
    #[error("validation helper I/O failed")]
    Io,
}

fn main() {
    if let Err(error) = run(env::args().skip(1).collect()) {
        eprintln!("validation helper failed: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), HelperError> {
    if !cfg!(target_arch = "x86_64") {
        return Err(HelperError::Platform);
    }
    let component_dir = parse_component_dir(&args)?;
    let component = litebubbles_validation_provider::validate_component_directory(component_dir)
        .map_err(|_| HelperError::Component)?;
    let mut generator = MacValidationGenerator::new(&component)?;
    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut output = stdout.lock();

    let initial: InitialPayload = read_json_line(&mut input)?;
    validate_input(&initial)?;
    let session_info = generator.initialize(initial.cert_data, initial.hardware_config)?;
    write_json_line(&mut output, &SessionInfoPayload { session_info })?;

    let apple_response: SessionInfoPayload = read_json_line(&mut input)?;
    if apple_response.session_info.len() > MAX_VALIDATION_BUFFER_BYTES {
        return Err(HelperError::Oversized);
    }
    let result = generator.key_establishment(apple_response.session_info)?;
    write_json_line(&mut output, &ResultPayload { result })?;
    Ok(())
}

fn parse_component_dir(args: &[String]) -> Result<PathBuf, HelperError> {
    if args.len() != 2 || args[0] != "--component-dir" {
        return Err(HelperError::Arguments);
    }
    Ok(PathBuf::from(&args[1]))
}

fn validate_input(initial: &InitialPayload) -> Result<(), HelperError> {
    if initial.cert_data.len() > MAX_VALIDATION_BUFFER_BYTES {
        return Err(HelperError::Oversized);
    }
    let encoded = serde_json::to_vec(initial).map_err(|_| HelperError::Malformed)?;
    if encoded.len() + 1 > MAX_JSON_LINE_BYTES {
        return Err(HelperError::Oversized);
    }
    Ok(())
}

fn read_json_line<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T, HelperError> {
    let mut line = Vec::new();
    let count = reader
        .read_until(b'\n', &mut line)
        .map_err(|_| HelperError::Io)?;
    if count == 0 || line.len() > MAX_JSON_LINE_BYTES {
        return Err(HelperError::Malformed);
    }
    serde_json::from_slice(&line).map_err(|_| HelperError::Malformed)
}

fn write_json_line<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), HelperError> {
    let encoded = serde_json::to_vec(value).map_err(|_| HelperError::Malformed)?;
    if encoded.len() + 1 > MAX_JSON_LINE_BYTES {
        return Err(HelperError::Oversized);
    }
    writer
        .write_all(&encoded)
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|_| HelperError::Io)
}

struct MacValidationGenerator {
    _library: Library,
    base_library_pointer: usize,
    validation_ctx_data_buffer: Option<Vec<u8>>,
}

impl MacValidationGenerator {
    fn new(component: &ValidatedComponent) -> Result<Self, HelperError> {
        // SAFETY: the component was hash- and manifest-validated immediately
        // before this call, and the helper is a process boundary for the
        // foreign library.
        let library = unsafe { Library::new(component.library_path()) }
            .map_err(|_| HelperError::Component)?;
        // SAFETY: the symbol name and ABI are part of the pinned public
        // v1.15.0+136 compatibility contract.
        let symbol: Symbol<unsafe extern "C" fn()> =
            unsafe { library.get(REFERENCE_SYMBOL) }.map_err(|_| HelperError::Abi)?;
        let reference_pointer =
            unsafe { symbol.try_as_raw_ptr() }.ok_or(HelperError::Abi)? as usize;
        let base_library_pointer = reference_pointer
            .checked_sub(REFERENCE_ADDRESS)
            .ok_or(HelperError::Abi)?;
        Ok(Self {
            _library: library,
            base_library_pointer,
            validation_ctx_data_buffer: None,
        })
    }

    fn initialize(
        &mut self,
        certs: Vec<u8>,
        hardware_config: HardwareConfig,
    ) -> Result<Vec<u8>, HelperError> {
        if certs.len() > MAX_VALIDATION_BUFFER_BYTES {
            return Err(HelperError::Oversized);
        }
        let function_address = self
            .base_library_pointer
            .checked_add(VALIDATION_CTX_NEW_ADDRESS)
            .ok_or(HelperError::Abi)?;
        // SAFETY: this function pointer and its arguments are the documented
        // ABI for the exact validated library release.
        let create_validation_context: unsafe extern "C" fn(
            *const c_void,
            *const c_void,
            *const c_void,
            *const c_void,
            *const c_void,
        ) = unsafe { transmute(function_address) };
        let mut session_info_buffer = vec![0_u8; MAX_VALIDATION_BUFFER_BYTES];
        let mut function_result_buffer = vec![0_u8; MAX_VALIDATION_BUFFER_BYTES];
        let cert_start = certs.as_ptr();
        let cert_end = cert_start.wrapping_add(certs.len());
        // SAFETY: all pointers remain valid for the duration of the call and
        // point to buffers with the documented capacities.
        unsafe {
            create_validation_context(
                function_result_buffer.as_mut_ptr() as *const c_void,
                cert_start as *const c_void,
                cert_end as *const c_void,
                session_info_buffer.as_mut_ptr() as *const c_void,
                &hardware_config as *const _ as *const c_void,
            );
        }
        let session_pointer = read_word(&session_info_buffer, 1)?;
        let session_length = read_word(&session_info_buffer, 2)? as usize;
        let session_info = copy_foreign_bytes(session_pointer, session_length)?;
        self.validation_ctx_data_buffer = Some(function_result_buffer);
        if session_info.is_empty() {
            return Err(HelperError::Malformed);
        }
        Ok(session_info)
    }

    fn key_establishment(&mut self, session_info: Vec<u8>) -> Result<Vec<u8>, HelperError> {
        if session_info.len() > MAX_VALIDATION_BUFFER_BYTES {
            return Err(HelperError::Oversized);
        }
        let function_address = self
            .base_library_pointer
            .checked_add(VALIDATION_CTX_KEY_ESTABLISHMENT_ADDRESS)
            .ok_or(HelperError::Abi)?;
        let key_establishment: unsafe extern "C" fn(
            *mut c_void,
            *mut c_void,
            *mut c_void,
            *mut c_void,
        ) -> *const c_void = unsafe { transmute(function_address) };
        let mut function_result_buffer = vec![0_u8; MAX_VALIDATION_BUFFER_BYTES];
        let mut context = self
            .validation_ctx_data_buffer
            .take()
            .ok_or(HelperError::Operation)?;
        // SAFETY: the pointers and lengths match the documented ABI; a crash
        // is contained by this helper process rather than litebubblesd.
        unsafe {
            key_establishment(
                function_result_buffer.as_mut_ptr() as *mut c_void,
                context.as_mut_ptr() as *mut c_void,
                session_info.as_ptr() as *mut c_void,
                session_info.len() as *mut c_void,
            );
        }
        self.validation_ctx_data_buffer = Some(context);
        self.sign()
    }

    fn sign(&mut self) -> Result<Vec<u8>, HelperError> {
        let function_address = self
            .base_library_pointer
            .checked_add(VALIDATION_CTX_SIGN_ADDRESS)
            .ok_or(HelperError::Abi)?;
        let sign: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const c_void =
            unsafe { transmute(function_address) };
        let mut function_result_buffer = vec![0_u8; MAX_VALIDATION_BUFFER_BYTES];
        let mut context = self
            .validation_ctx_data_buffer
            .take()
            .ok_or(HelperError::Operation)?;
        // SAFETY: the pointers match the documented ABI and the foreign call
        // is isolated in this short-lived process.
        unsafe {
            sign(
                function_result_buffer.as_mut_ptr() as *mut c_void,
                context.as_mut_ptr() as *mut c_void,
            );
        }
        self.validation_ctx_data_buffer = Some(context);
        let result_pointer = read_word(&function_result_buffer, 2)?;
        let result_length = read_word(&function_result_buffer, 3)? as usize;
        let result = copy_foreign_bytes(result_pointer, result_length)?;
        if result.is_empty() {
            return Err(HelperError::Malformed);
        }
        Ok(result)
    }
}

fn read_word(buffer: &[u8], index: usize) -> Result<u64, HelperError> {
    let start = index.checked_mul(8).ok_or(HelperError::Malformed)?;
    let end = start.checked_add(8).ok_or(HelperError::Malformed)?;
    let bytes = buffer.get(start..end).ok_or(HelperError::Malformed)?;
    let bytes: [u8; 8] = bytes.try_into().map_err(|_| HelperError::Malformed)?;
    Ok(u64::from_ne_bytes(bytes))
}

fn copy_foreign_bytes(pointer: u64, length: usize) -> Result<Vec<u8>, HelperError> {
    if length > MAX_VALIDATION_BUFFER_BYTES {
        return Err(HelperError::Oversized);
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer == 0 {
        return Err(HelperError::Malformed);
    }
    let mut bytes = vec![0_u8; length];
    // SAFETY: the foreign component supplied the pointer and length.  The
    // process boundary limits the consequence of a bad ABI or pointer.
    unsafe {
        std::ptr::copy_nonoverlapping(pointer as *const u8, bytes.as_mut_ptr(), length);
    }
    Ok(bytes)
}
