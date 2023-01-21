use super::*;
/// File generated by fuel-core: benches/src/bin/collect.rs:440. With the following git hash
pub const GIT: &str = "99280c3f8a9edcf509ac15b096d73f0f0b86a261";
pub fn default_gas_costs() -> GasCostsValues {
    GasCostsValues {
        add: 2,
        addi: 1,
        aloc: 1,
        and: 1,
        andi: 1,
        bal: 1,
        bhei: 1,
        bhsh: 1,
        burn: 1,
        cb: 1,
        cfei: 1,
        cfsi: 1,
        croo: 1,
        div: 1,
        divi: 1,
        ecr: 1,
        eq: 1,
        exp: 1,
        expi: 1,
        flag: 1,
        gm: 1,
        gt: 1,
        gtf: 1,
        ji: 1,
        jmp: 1,
        jne: 1,
        jnei: 1,
        jnzi: 1,
        k256: 1,
        lb: 1,
        log: 1,
        lt: 1,
        lw: 1,
        mcpi: 1,
        mint: 1,
        mlog: 1,
        srwq: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        modi: 1,
        mod_op: 1,
        movi: 1,
        mroo: 1,
        mul: 1,
        muli: 1,
        noop: 1,
        not: 1,
        or: 1,
        ori: 1,
        move_op: 1,
        ret: 1,
        s256: 1,
        sb: 1,
        scwq: 1,
        sll: 1,
        slli: 1,
        srl: 1,
        srli: 1,
        srw: 1,
        sub: 1,
        subi: 1,
        sw: 1,
        sww: 1,
        swwq: 1,
        time: 1,
        tr: 1,
        tro: 1,
        xor: 1,
        xori: 1,
        call: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        ccp: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        csiz: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        ldc: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        logd: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        mcl: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        mcli: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        mcp: DependentCost {
            base: 10,
            dep_per_unit: 1475,
        },
        meq: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        rvrt: 1,
        smo: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
        retd: DependentCost {
            base: 1,
            dep_per_unit: 0,
        },
    }
}
