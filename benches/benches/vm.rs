mod utils;
mod vm_set;

use std::time::Duration;

use criterion::{
    black_box,
    criterion_group,
    criterion_main,
    measurement::WallTime,
    BenchmarkGroup,
    Criterion,
};

use fuel_core_benches::*;
use fuel_core_storage::transactional::Transaction;
use fuel_core_types::fuel_asm::Instruction;
use vm_set::*;

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

            let clock = quanta::Clock::new();

            let final_time;
            loop {
                // Measure the total time to revert the VM to the initial state.
                // It should always do the same things regardless of the number of
                // iterations because we use a `diff` from the `VmBenchPrepared` initialization.
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = black_box(clock.raw());
                    vm.reset_vm_state(diff);
                    let end = black_box(clock.raw());
                    total += clock.delta(start, end);
                    black_box(&vm);
                }
                let time_to_reset = total;

                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = black_box(clock.raw());
                    match instruction {
                        Instruction::CALL(call) => {
                            let (ra, rb, rc, rd) = call.unpack();
                            black_box(vm.prepare_call(ra, rb, rc, rd)).unwrap();
                        }
                        _ => {
                            black_box(vm.instruction(*instruction).unwrap());
                        }
                    }
                    black_box(&vm);
                    let end = black_box(clock.raw());
                    total += clock.delta(start, end);
                    vm.reset_vm_state(diff);
                }
                let only_instruction = total.checked_sub(time_to_reset);

                // It may overflow when the benchmarks run in an unstable environment.
                // If the hardware is busy during the measuring time to reset the VM,
                // it will produce `time_to_reset` more than the actual time
                // to run the instruction and reset the VM.
                if let Some(result) = only_instruction {
                    final_time = result;
                    break
                } else {
                    println!("The environment is unstable. Rerunning the benchmark.");
                }
            }

            db_txn.commit().unwrap();
            // restore original db
            *vm.as_mut().database_mut() = original_db;
            final_time
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
