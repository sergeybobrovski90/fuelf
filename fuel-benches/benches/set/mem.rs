use super::run_group_ref;

use criterion::Criterion;
use fuel_core_benches::*;

pub fn run(c: &mut Criterion) {
    let mut group = c.benchmark_group("mem");

    let linear = vec![1, 10, 100, 1_000, 10_000, 100_000];
    // let linear = vec![100_000];
    // let linear = vec![1, 10, 100, 1_000, 10_000];
    // let linear = vec![1, 10, 25, 50, 75, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1_000];

    run_group_ref(
        &mut group,
        format!("aloc ({})", 1),
        VmBench::new(Opcode::ALOC(0x10)).with_prepare_script(vec![Opcode::MOVI(0x10, 1)]),
    );

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("cfei ({})", i),
            VmBench::new(Opcode::CFEI(*i)).with_cleanup(vec![Opcode::CFSI(*i)]),
        );
    }

    run_group_ref(
        &mut group,
        "lb",
        VmBench::new(Opcode::LB(0x10, REG_ONE, 10)),
    );

    run_group_ref(
        &mut group,
        "lw",
        VmBench::new(Opcode::LW(0x10, REG_ONE, 10)),
    );

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("mcl ({})", i),
            VmBench::new(Opcode::MCL(0x10, 0x11)).with_prepare_script(vec![
                Opcode::MOVI(0x11, *i),
                Opcode::ALOC(0x11),
                Opcode::ADDI(0x10, REG_HP, 1),
            ]),
        );
    }

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("mcli ({})", i),
            VmBench::new(Opcode::MCLI(0x10, *i)).with_prepare_script(vec![
                Opcode::MOVI(0x11, *i),
                Opcode::ALOC(0x11),
                Opcode::ADDI(0x10, REG_HP, 1),
            ]),
        );
    }

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("mcp {}", i),
            VmBench::new(Opcode::MCP(0x10, REG_ZERO, 0x11)).with_prepare_script(vec![
                Opcode::MOVI(0x11, *i),
                Opcode::ALOC(0x11),
                Opcode::ADDI(0x10, REG_HP, 1),
            ]),
        );
    }

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("mcpi ({})", i),
            VmBench::new(Opcode::MCPI(0x10, REG_ZERO, *i as Immediate12))
                .with_prepare_script(vec![
                    Opcode::MOVI(0x11, *i),
                    Opcode::ALOC(0x11),
                    Opcode::ADDI(0x10, REG_HP, 1),
                ]),
        );
    }

    for i in &linear {
        run_group_ref(
            &mut group,
            format!("meq ({})", i),
            VmBench::new(Opcode::MEQ(0x10, 0x11, 0x12, 0x13)).with_prepare_script(vec![
                Opcode::MOVI(0x11, 0),
                Opcode::MOVI(0x12, i * 3),
                Opcode::MOVI(0x13, *i),
            ]),
        );
    }

    run_group_ref(
        &mut group,
        "sb",
        VmBench::new(Opcode::SB(0x10, 0x11, 0)).with_prepare_script(vec![
            Opcode::ALOC(REG_ONE),
            Opcode::ADDI(0x10, REG_HP, 1),
            Opcode::MOVI(0x11, 50),
        ]),
    );

    run_group_ref(
        &mut group,
        "sw",
        VmBench::new(Opcode::SW(0x10, 0x11, 0)).with_prepare_script(vec![
            Opcode::MOVI(0x10, 8),
            Opcode::ALOC(0x10),
            Opcode::ADDI(0x10, REG_HP, 1),
            Opcode::MOVI(0x11, 50),
        ]),
    );

    group.finish();
}
