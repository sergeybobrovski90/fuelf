use fuel_core_executor::executor::ExecutionBlockWithSource;
use fuel_core_storage::transactional::Changes;
use fuel_core_types::services::{
    executor::{
        ExecutionResult,
        Result as ExecutorResult,
    },
    Uncommitted,
};

/// Pack a pointer and length into an `u64`.
pub fn pack_ptr_and_len(ptr: u32, len: u32) -> u64 {
    (u64::from(len) << 32) | u64::from(ptr)
}

/// Unpacks an `u64` into the pointer and length.
pub fn unpack_ptr_and_len(val: u64) -> (u32, u32) {
    let ptr = u32::try_from(val & (u32::MAX as u64))
        .expect("It ony contains first 32 bytes; qed");
    let len = u32::try_from(val >> 32).expect("It ony contains first 32 bytes; qed");

    (ptr, len)
}

/// Pack a `exists`, `size` and `result` into one `u64`.
pub fn pack_exists_size_result(exists: bool, size: u32, result: u16) -> u64 {
    (u64::from(result) << 33 | u64::from(size) << 1) | u64::from(exists)
}

/// Unpacks an `u64` into `exists`, `size` and `result`.
pub fn unpack_exists_size_result(val: u64) -> (bool, u32, u16) {
    let exists = (val & 1u64) != 0;
    let size = u32::try_from((val >> 1) & (u32::MAX as u64))
        .expect("It only contains first 32 bytes; qed");
    let result = u16::try_from(val >> 33 & (u16::MAX as u64))
        .expect("It only contains first 16 bytes; qed");

    (exists, size, result)
}

pub type InputType = ExecutionBlockWithSource<()>;

pub type ReturnType = ExecutorResult<Uncommitted<ExecutionResult, Changes>>;
