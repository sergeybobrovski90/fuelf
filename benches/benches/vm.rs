mod set;
mod utils;

use criterion::{
    black_box,
    criterion_group,
    criterion_main,
    measurement::WallTime,
    BenchmarkGroup,
    Criterion,
};
use std::time::Duration;

use fuel_core_benches::*;
use fuel_core_storage::transactional::Transaction;
use fuel_core_types::fuel_asm::Instruction;
use set::*;

// Use Jemalloc during benchmarks
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

pub fn run_group_ref<I>(group: &mut BenchmarkGroup<WallTime>, id: I, bench: VmBench)
where
    I: AsRef<str>,
{
    let mut i = bench.prepare().expect("failed to prepare bench");
    group.bench_function::<_, _>(id.as_ref(), move |b| {
        b.iter_custom(|iters| {
            let VmBenchPrepared {
                vm,
                instruction,
                diff,
            } = &mut i;
            let original_db = vm.as_mut().database_mut().clone();
            let mut db_txn = {
                let db = vm.as_mut().database_mut();
                let db_txn = db.transaction();
                // update vm database in-place to use transaction
                *db = db_txn.as_ref().clone();
                db_txn
            };

            let mut elapsed_time = Duration::default();
            for _ in 0..iters {
                let start = std::time::Instant::now();
                match instruction {
                    Instruction::CALL(call) => {
                        let (ra, rb, rc, rd) = call.unpack();
                        vm.prepare_call(ra, rb, rc, rd).unwrap();
                    }
                    _ => {
                        black_box(vm.instruction(*instruction).unwrap());
                    }
                }
                elapsed_time += start.elapsed();
                vm.reset_vm_state(diff);
            }
            db_txn.commit().unwrap();
            // restore original db
            *vm.as_mut().database_mut() = original_db;
            elapsed_time
        })
    });
}
fn vm(c: &mut Criterion) {
    alu::run(c);
    blockchain::run(c);
    crypto::run(c);
    flow::run(c);
    mem::run(c);
}

criterion_group!(benches, vm);
criterion_main!(benches);
