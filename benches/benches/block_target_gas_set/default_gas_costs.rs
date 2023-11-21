use super::*;
pub fn default_gas_costs() -> GasCostsValues {
    GasCostsValues {
        add: 2,
        addi: 2,
        aloc: 1,
        and: 2,
        andi: 2,
        bal: 328,
        bhei: 1,
        bhsh: 2,
        burn: 27738,
        cb: 2,
        cfei: 2,
        cfsi: 1,
        croo: 40,
        div: 2,
        divi: 2,
        eck1: 3107,
        ecr1: 42738,
        ed19: 2897,
        eq: 2,
        exp: 2,
        expi: 2,
        flag: 1,
        gm: 2,
        gt: 2,
        gtf: 2,
        ji: 2,
        jmp: 2,
        jne: 2,
        jnei: 2,
        jnzi: 2,
        jmpf: 1,
        jmpb: 1,
        jnzf: 1,
        jnzb: 1,
        jnef: 1,
        jneb: 1,
        lb: 2,
        log: 87,
        lt: 2,
        lw: 2,
        mint: 25515,
        mlog: 2,
        vm_initialization: 1,
        modi: 2,
        mod_op: 2,
        movi: 2,
        mroo: 4,
        mul: 2,
        muli: 2,
        mldv: 4,
        noop: 1,
        not: 1,
        or: 2,
        ori: 2,
        poph: 3,
        popl: 3,
        pshh: 3,
        pshl: 3,
        move_op: 1,
        ret: 127,
        sb: 2,
        sll: 2,
        slli: 2,
        srl: 2,
        srli: 2,
        srw: 224,
        sub: 2,
        subi: 2,
        sw: 2,
        sww: 26247,
        time: 76,
        tr: 38925,
        tro: 26756,
        wdcm: 2,
        wqcm: 3,
        wdop: 3,
        wqop: 3,
        wdml: 3,
        wqml: 4,
        wddv: 5,
        wqdv: 6,
        wdmd: 10,
        wqmd: 17,
        wdam: 9,
        wqam: 10,
        wdmm: 10,
        wqmm: 10,
        xor: 2,
        xori: 2,
        call: DependentCost::LightOperation {
            base: 17510,
            units_per_gas: 5,
        },
        ccp: DependentCost::LightOperation {
            base: 54,
            units_per_gas: 21,
        },
        csiz: DependentCost::LightOperation {
            base: 58,
            units_per_gas: 212,
        },
        k256: DependentCost::LightOperation {
            base: 259,
            units_per_gas: 4,
        },
        ldc: DependentCost::LightOperation {
            base: 42,
            units_per_gas: 65,
        },
        logd: DependentCost::LightOperation {
            base: 413,
            units_per_gas: 3,
        },
        mcl: DependentCost::LightOperation {
            base: 2,
            units_per_gas: 568,
        },
        mcli: DependentCost::LightOperation {
            base: 3,
            units_per_gas: 568,
        },
        mcp: DependentCost::LightOperation {
            base: 3,
            units_per_gas: 470,
        },
        mcpi: DependentCost::LightOperation {
            base: 6,
            units_per_gas: 682,
        },
        meq: DependentCost::LightOperation {
            base: 10,
            units_per_gas: 1161,
        },
        rvrt: 127,
        s256: DependentCost::LightOperation {
            base: 42,
            units_per_gas: 3,
        },
        scwq: DependentCost::HeavyOperation {
            base: 27337,
            gas_per_unit: 25552,
        },
        smo: DependentCost::LightOperation {
            base: 55851,
            units_per_gas: 1,
        },
        srwq: DependentCost::HeavyOperation {
            base: 501,
            gas_per_unit: 22,
        },
        swwq: DependentCost::HeavyOperation {
            base: 25619,
            gas_per_unit: 24002,
        },
        contract_root: DependentCost::LightOperation {
            base: 43,
            units_per_gas: 2,
        },
        state_root: DependentCost::HeavyOperation {
            base: 324,
            gas_per_unit: 164,
        },
        new_storage_per_byte: 1,
        retd: DependentCost::LightOperation {
            base: 434,
            units_per_gas: 3,
        },
    }
}
