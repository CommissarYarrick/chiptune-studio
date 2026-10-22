//! Minimal WebAssembly ABI; the browser owns all inputs and outputs in memory.
//! No JavaScript binding generator, services, or additional dependencies.

#[unsafe(no_mangle)]
pub extern "C" fn chip_alloc(length: u32) -> *mut u8 {
    if length > 20 * 1024 * 1024 {
        return std::ptr::null_mut();
    }
    let bytes = vec![0u8; length as usize].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn chip_free(pointer: *mut u8, length: u32) {
    if !pointer.is_null() {
        unsafe {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                pointer,
                length as usize,
            )));
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn chip_run(pointer: *const u8, length: u32) -> u64 {
    let result = (|| -> anyhow::Result<(u8, Vec<u8>)> {
        anyhow::ensure!(
            length <= 20 * 1024 * 1024 && !pointer.is_null(),
            "Invalid input"
        );
        let bytes = unsafe { std::slice::from_raw_parts(pointer, length as usize) };
        let request = serde_json::from_slice(bytes)?;
        Ok(match crate::browser::run(&request)? {
            crate::browser::Output::Json(value) => (1, serde_json::to_vec(&value)?),
            crate::browser::Output::Wave(bytes) => (2, bytes),
        })
    })();
    let (kind, body) = result.unwrap_or_else(|e| (0, e.to_string().into_bytes()));
    let mut output = Vec::with_capacity(body.len() + 1);
    output.push(kind);
    output.extend(body);
    let bytes = output.into_boxed_slice();
    let length = bytes.len() as u64;
    let pointer = Box::into_raw(bytes) as *mut u8 as u64;
    (length << 32) | pointer
}
