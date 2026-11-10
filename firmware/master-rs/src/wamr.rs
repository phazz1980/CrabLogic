//! Small, platform-neutral WAMR configuration shared by the board runtime.

use alloc::string::String;

/// Execution backend selected for one module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// WAMR AOT image executed directly from flash (XIP).
    AotXip,
    /// Classic WAMR interpreter; bytecode is copied to RAM.
    Interpreter,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AotXip => "aot",
            Self::Interpreter => "interp",
        }
    }
}

/// Metadata needed before creating a WAMR instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSpec {
    pub name: String,
    pub mode: ExecutionMode,
    pub max_memory: usize,
    pub stack_size: usize,
}

impl ModuleSpec {
    pub fn new(name: String, mode: ExecutionMode) -> Self {
        Self {
            name,
            mode,
            max_memory: 64 * 1024,
            stack_size: 4096,
        }
    }

    /// Validate limits before handing them to WAMR.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.is_empty() {
            return Err("пустое имя модуля");
        }
        if self.max_memory == 0 || !self.max_memory.is_multiple_of(65536) {
            return Err("память модуля должна быть кратна странице WASM");
        }
        if self.stack_size < 1024 {
            return Err("стек WASM слишком мал");
        }
        Ok(())
    }

    /// Select the on-flash artifact expected by the selected backend.
    pub fn artifact_suffix(&self) -> &'static str {
        match self.mode {
            ExecutionMode::AotXip => ".aot",
            ExecutionMode::Interpreter => ".wasm",
        }
    }
}

fn leb(bytes: &[u8], pos: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// Bytes of linear memory a module needs: its custom section `crab.mem` (u32 LE,
/// `specs/mcu-memory`). `None` if `wasm` is not a module or has no such section.
pub fn crab_mem(wasm: &[u8]) -> Option<u32> {
    if !wasm.starts_with(b"\0asm") {
        return None;
    }
    let mut pos = 8;
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let size = leb(wasm, &mut pos)? as usize;
        let section = wasm.get(pos..pos.checked_add(size)?)?;
        pos += size;
        if id != 0 {
            continue;
        }
        let mut at = 0;
        let name_len = leb(section, &mut at)? as usize;
        let name = section.get(at..at.checked_add(name_len)?)?;
        let data = &section[at + name_len..];
        if name == b"crab.mem" && data.len() == 4 {
            return Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crab_mem_is_read_from_the_custom_section() {
        let mut wasm = alloc::vec![0, b'a', b's', b'm', 1, 0, 0, 0];
        // A type section to skip, then `crab.mem` = 4096.
        wasm.extend_from_slice(&[1, 1, 0]);
        wasm.extend_from_slice(&[0, 13, 8]);
        wasm.extend_from_slice(b"crab.mem");
        wasm.extend_from_slice(&4096u32.to_le_bytes());
        assert_eq!(crab_mem(&wasm), Some(4096));
        assert_eq!(crab_mem(&wasm[..11]), None);
        assert_eq!(crab_mem(&wasm[..20]), None);
        assert_eq!(crab_mem(b"\0aot"), None);
    }

    #[test]
    fn modes_are_protocol_stable() {
        assert_eq!(ExecutionMode::AotXip.as_str(), "aot");
        assert_eq!(ExecutionMode::Interpreter.as_str(), "interp");
    }

    #[test]
    fn default_module_limits_match_c3_stage_zero() {
        let spec = ModuleSpec::new(String::from("blink"), ExecutionMode::AotXip);
        assert_eq!(spec.max_memory, 64 * 1024);
        assert_eq!(spec.stack_size, 4096);
        assert_eq!(spec.artifact_suffix(), ".aot");
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn invalid_limits_are_rejected_before_wamr() {
        let mut spec = ModuleSpec::new(String::from("blink"), ExecutionMode::Interpreter);
        spec.max_memory = 1;
        assert_eq!(
            spec.validate(),
            Err("память модуля должна быть кратна странице WASM")
        );
        spec.max_memory = 65536;
        spec.stack_size = 512;
        assert_eq!(spec.validate(), Err("стек WASM слишком мал"));
        assert_eq!(spec.artifact_suffix(), ".wasm");
    }
}
