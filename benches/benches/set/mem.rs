use super::run_group_ref;

use criterion::{
    Criterion,
    Throughput,
};
use fuel_core_benches::*;
use fuel_core_types::fuel_asm::*;

/// Set a register `r` to a Word-sized number value using left-shifts
fn set_full_word(r: RegisterId, v: Word) -> Vec<Instruction> {
    let r = u8::try_from(r).unwrap();
    let mut ops = vec![op::movi(r, 0)];
    for byte in v.to_be_bytes() {
        ops.push(op::ori(r, r, byte as u16));
        ops.push(op::slli(r, r, 8));
    }
    ops.pop().unwrap(); // Remove last shift
    ops
}

pub fn run(c: &mut Criterion) {
    run_group_ref(
        &mut c.benchmark_group("lb"),
        "lb",
        VmBench::new(op::lb(0x10, RegId::ONE, 10)),
    );

    run_group_ref(
        &mut c.benchmark_group("lw"),
        "lw",
        VmBench::new(op::lw(0x10, RegId::ONE, 10)),
    );

    run_group_ref(
        &mut c.benchmark_group("sb"),
        "sb",
        VmBench::new(op::sb(0x10, 0x11, 0)).with_prepare_script(vec![
            op::aloc(RegId::ONE),
            op::move_(0x10, RegId::HP),
            op::movi(0x11, 50),
        ]),
    );

    run_group_ref(
        &mut c.benchmark_group("sw"),
        "sw",
        VmBench::new(op::sw(0x10, 0x11, 0)).with_prepare_script(vec![
            op::movi(0x10, 8),
            op::aloc(0x10),
            op::move_(0x10, RegId::HP),
            op::movi(0x11, 50),
        ]),
    );

    let linear = super::generate_linear_costs();

    run_group_ref(
        &mut c.benchmark_group("cfei"),
        "cfei",
        VmBench::new(op::cfei(1)),
    );

    let mut mem_mcl = c.benchmark_group("mcl");
    for i in &linear {
        mem_mcl.throughput(Throughput::Bytes(*i as u64));
        run_group_ref(
            &mut mem_mcl,
            format!("{i}"),
            VmBench::new(op::mcl(0x10, 0x11)).with_prepare_script(vec![
                op::movi(0x11, *i),
                op::aloc(0x11),
                op::move_(0x10, RegId::HP),
            ]),
        );
    }
    mem_mcl.finish();

    let mut mem_mcli = c.benchmark_group("mcli");
    for i in &linear {
        mem_mcli.throughput(Throughput::Bytes(*i as u64));
        run_group_ref(
            &mut mem_mcli,
            format!("{i}"),
            VmBench::new(op::mcli(0x10, *i)).with_prepare_script(vec![
                op::movi(0x11, *i),
                op::aloc(0x11),
                op::move_(0x10, RegId::HP),
            ]),
        );
    }
    mem_mcli.finish();

    let mut mem_mcp = c.benchmark_group("mcp");
    for i in &linear {
        mem_mcp.throughput(Throughput::Bytes(*i as u64));
        run_group_ref(
            &mut mem_mcp,
            format!("{i}"),
            VmBench::new(op::mcp(0x10, RegId::ZERO, 0x11)).with_prepare_script(vec![
                op::movi(0x11, *i),
                op::aloc(0x11),
                op::move_(0x10, RegId::HP),
            ]),
        );
    }
    mem_mcp.finish();

    let mut mem_mcpi = c.benchmark_group("mcp");

    let mut imm12_linear: Vec<_> = linear
        .iter()
        .copied()
        .take_while(|p| *p < (1 << 12))
        .collect();
    imm12_linear.push((1 << 12) - 1);
    for i in &imm12_linear {
        let i_as_u16: u16 = (*i).try_into().unwrap();
        mem_mcpi.throughput(Throughput::Bytes(*i as u64));
        run_group_ref(
            &mut mem_mcpi,
            format!("{i}"),
            VmBench::new(op::mcpi(0x10, RegId::ZERO, i_as_u16)).with_prepare_script(
                vec![
                    op::movi(0x11, *i),
                    op::aloc(0x11),
                    op::move_(0x10, RegId::HP),
                ],
            ),
        );
    }
    mem_mcpi.finish();

    let mut mem_meq = c.benchmark_group("meq");
    for i in &linear {
        let i = *i as u64;
        mem_meq.throughput(Throughput::Bytes(i));

        let mut prepare_script =
            vec![op::move_(0x11, RegId::ZERO), op::move_(0x12, RegId::ZERO)];
        prepare_script.extend(set_full_word(0x13, i));

        run_group_ref(
            &mut mem_meq,
            format!("{i}"),
            VmBench::new(op::meq(0x10, 0x11, 0x12, 0x13))
                .with_prepare_script(prepare_script),
        );
    }
    mem_meq.finish();
}
