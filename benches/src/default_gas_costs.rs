use super::*;
use fuel_core_types::fuel_tx::consensus_parameters::gas::GasCostsValuesV3;
pub fn default_gas_costs() -> GasCostsValues {
    GasCostsValuesV3 {
        add: 2,
        addi: 2,
        and: 2,
        andi: 2,
        bal: 29,
        bhei: 2,
        bhsh: 2,
        burn: 19976,
        cb: 2,
        cfsi: 2,
        div: 2,
        divi: 2,
        eck1: 1907,
        ecr1: 26135,
        ed19: 1893,
        eq: 2,
        exp: 2,
        expi: 2,
        flag: 2,
        gm: 2,
        gt: 2,
        gtf: 13,
        ji: 2,
        jmp: 2,
        jne: 2,
        jnei: 2,
        jnzi: 2,
        jmpf: 2,
        jmpb: 2,
        jnzf: 2,
        jnzb: 2,
        jnef: 2,
        jneb: 2,
        lb: 2,
        log: 102,
        lt: 2,
        lw: 2,
        mint: 18042,
        mlog: 2,
        vm_initialization: DependentCost::LightOperation {
            base: 3957,
            units_per_gas: 48,
        },
        modi: 2,
        mod_op: 2,
        movi: 2,
        mroo: 4,
        mul: 2,
        muli: 2,
        mldv: 3,
        noop: 1,
        not: 2,
        or: 2,
        ori: 2,
        poph: 3,
        popl: 3,
        pshh: 5,
        pshl: 5,
        move_op: 2,
        ret: 53,
        sb: 2,
        sll: 2,
        slli: 2,
        srl: 2,
        srli: 2,
        srw: 177,
        sub: 2,
        subi: 2,
        sw: 2,
        sww: 17302,
        time: 35,
        tr: 27852,
        tro: 19718,
        wdcm: 2,
        wqcm: 2,
        wdop: 3,
        wqop: 3,
        wdml: 3,
        wqml: 3,
        wddv: 4,
        wqdv: 5,
        wdmd: 8,
        wqmd: 12,
        wdam: 7,
        wqam: 8,
        wdmm: 8,
        wqmm: 8,
        xor: 2,
        xori: 2,
        aloc: DependentCost::LightOperation {
            base: 2,
            units_per_gas: 15,
        },
        cfe: DependentCost::LightOperation {
            base: 10,
            units_per_gas: 1818181,
        },
        cfei: DependentCost::LightOperation {
            base: 2,
            units_per_gas: 1000000,
        },
        call: DependentCost::LightOperation {
            base: 13513,
            units_per_gas: 7,
        },
        ccp: DependentCost::LightOperation {
            base: 34,
            units_per_gas: 39,
        },
        croo: DependentCost::LightOperation {
            base: 91,
            units_per_gas: 3,
        },
        csiz: DependentCost::LightOperation {
            base: 31,
            units_per_gas: 438,
        },
        k256: DependentCost::LightOperation {
            base: 27,
            units_per_gas: 5,
        },
        ldc: DependentCost::LightOperation {
            base: 43,
            units_per_gas: 102,
        },
        logd: DependentCost::LightOperation {
            base: 363,
            units_per_gas: 4,
        },
        mcl: DependentCost::LightOperation {
            base: 2,
            units_per_gas: 1041,
        },
        mcli: DependentCost::LightOperation {
            base: 2,
            units_per_gas: 1025,
        },
        mcp: DependentCost::LightOperation {
            base: 4,
            units_per_gas: 325,
        },
        mcpi: DependentCost::LightOperation {
            base: 8,
            units_per_gas: 511,
        },
        meq: DependentCost::LightOperation {
            base: 3,
            units_per_gas: 940,
        },
        rvrt: 52,
        s256: DependentCost::LightOperation {
            base: 31,
            units_per_gas: 4,
        },
        scwq: DependentCost::HeavyOperation {
            base: 16346,
            gas_per_unit: 17163,
        },
        smo: DependentCost::LightOperation {
            base: 40860,
            units_per_gas: 2,
        },
        srwq: DependentCost::HeavyOperation {
            base: 187,
            gas_per_unit: 179,
        },
        swwq: DependentCost::HeavyOperation {
            base: 17046,
            gas_per_unit: 16232,
        },
        contract_root: DependentCost::LightOperation {
            base: 31,
            units_per_gas: 2,
        },
        state_root: DependentCost::HeavyOperation {
            base: 236,
            gas_per_unit: 122,
        },
        new_storage_per_byte: 63,
        retd: DependentCost::LightOperation {
            base: 305,
            units_per_gas: 4,
        },
    }
    .into()
}
