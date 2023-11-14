use crate::{
    utils::arb_dependent_cost_values,
    *,
};
use fuel_core_types::{
    fuel_types::{
        Address,
        Word,
    },
    fuel_vm::consts::WORD_SIZE,
};

// BAL: Balance of contract ID
// BHEI: Block height
// BHSH: Block hash
// BURN: Burn existing coins
// CALL: Call contract
// CB: Coinbase address
// CCP: Code copy
// CROO: Code Merkle root
// CSIZ: Code size
// LDC: Load code from an external contract
// LOG: Log event
// LOGD: Log data event
// MINT: Mint new coins
// RETD: Return from context with data
// RVRT: Revert
// SMO: Send message to output
// SCWQ: State clear sequential 32 byte slots
// SRW: State read word
// SRWQ: State read sequential 32 byte slots
// SWW: State write word
// SWWQ: State write sequential 32 byte slots
// TIME: Timestamp at height
// TR: Transfer coins to contract
// TRO: Transfer coins to output
pub fn run_contract(group: &mut BenchmarkGroup<WallTime>) {
    let contract_id = ContractId::zeroed();
    let asset_id = AssetId::zeroed();
    let script_data = script_data(&contract_id, &asset_id);

    let mut shared_runner_builder = SanityBenchmarkRunnerBuilder::new_shared(contract_id);

    // bal contract
    {
        let contract_instructions =
            vec![op::bal(0x13, 0x11, 0x10), op::jmpb(RegId::ZERO, 0)];

        let mut instructions = setup_instructions();
        instructions.extend(vec![op::call(0x10, RegId::ZERO, 0x11, 0x12)]);
        let id = "contract/bal contract";

        shared_runner_builder
            .build_with_new_contract(contract_instructions)
            .run(id, group, instructions, script_data.clone());
    }

    {
        let mut instructions = setup_instructions();
        instructions.extend(vec![op::bal(0x13, 0x11, 0x10), op::jmpb(RegId::ZERO, 0)]);
        let id = "contract/bal script";
        shared_runner_builder
            .build()
            .run(id, group, instructions, script_data.clone());
    }

    // bhei
    run(
        "contract/bhei",
        group,
        vec![op::bhei(0x10), op::jmpb(RegId::ZERO, 0)],
        vec![],
    );

    // bhsh
    run(
        "contract/bhsh",
        group,
        vec![
            op::movi(0x10, Bytes32::LEN.try_into().unwrap()),
            op::aloc(0x10),
            op::move_(0x10, RegId::HP),
            op::bhsh(0x10, RegId::ZERO),
            op::jmpb(RegId::ZERO, 0),
        ],
        vec![],
    );

    // burn
    {
        let contract = vec![op::burn(RegId::ONE, RegId::HP), op::jmpb(RegId::ZERO, 0)];
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x10, 32),
            op::aloc(0x10),
            op::call(0x10, RegId::ZERO, 0x11, 0x12),
        ]);
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/burn",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // call
    for size in arb_dependent_cost_values() {
        let mut contract_instructions = std::iter::repeat(op::noop())
            .take(size as usize)
            .collect::<Vec<_>>();
        contract_instructions.push(op::ret(0x10));

        let instructions = vec![
            op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
            op::addi(0x11, 0x10, ContractId::LEN.try_into().unwrap()),
            op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
            op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
            op::movi(0x12, TARGET_BLOCK_GAS_LIMIT as u32),
            op::call(0x10, RegId::ZERO, 0x11, 0x12),
            op::jmpb(RegId::ZERO, 0),
        ];

        let id = format!("contract/call {:?}", size);
        shared_runner_builder
            .build_with_new_contract(contract_instructions)
            .run(&id, group, instructions, script_data.clone());
    }

    // cb
    {
        run(
            "contract/cb",
            group,
            vec![
                op::movi(0x10, Bytes32::LEN.try_into().unwrap()),
                op::aloc(0x10),
                op::move_(0x10, RegId::HP),
                op::cb(0x10),
                op::jmpb(RegId::ZERO, 0),
            ],
            vec![],
        );
    }

    // ccp
    for i in arb_dependent_cost_values() {
        let contract = std::iter::repeat(op::noop())
            .take(i as usize)
            .chain(vec![op::ret(RegId::ZERO)])
            .collect();

        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x13, i),
            op::movi(0x14, i),
            op::movi(0x15, i),
            op::add(0x15, 0x15, 0x15),
            op::addi(0x15, 0x15, 32),
            op::aloc(0x15),
            op::move_(0x15, RegId::HP),
            op::ccp(0x15, 0x10, RegId::ZERO, 0x13),
            op::jmpb(RegId::ZERO, 0),
        ]);
        let id = format!("contract/ccp {:?}", i);
        shared_runner_builder.build_with_new_contract(contract).run(
            &id,
            group,
            instructions,
            script_data.clone(),
        );
    }
    // croo
    {
        let contract = vec![
            op::gtf_args(0x16, 0x00, GTFArgs::ScriptData),
            op::movi(0x15, 32),
            op::aloc(0x15),
            op::move_(0x14, RegId::HP),
            op::croo(0x14, 0x16),
            op::ret(RegId::ZERO),
        ];
        let instructions = call_contract_repeat();
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/croo",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // csiz
    for size in arb_dependent_cost_values() {
        let contract = std::iter::repeat(op::noop())
            .take(size as usize)
            .chain(vec![op::ret(RegId::ZERO)])
            .collect();
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
            op::csiz(0x11, 0x10),
            op::jmpb(RegId::ZERO, 0),
        ]);
        let id = format!("contract/csiz {:?}", size);
        shared_runner_builder.build_with_new_contract(contract).run(
            &id,
            group,
            instructions,
            script_data.clone(),
        );
    }

    // ldc
    for size in arb_dependent_cost_values() {
        let contract = std::iter::repeat(op::noop())
            .take(size as usize)
            .chain(vec![op::ret(RegId::ZERO)])
            .collect();
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x13, size),
            op::ldc(0x10, RegId::ZERO, 0x13),
            op::jmpb(RegId::ZERO, 0),
        ]);
        let id = format!("contract/ldc {:?}", size);
        shared_runner_builder.build_with_new_contract(contract).run(
            &id,
            group,
            instructions,
            script_data.clone(),
        );
    }

    // log
    {
        run(
            "contract/log",
            group,
            vec![op::log(0x10, 0x11, 0x12, 0x13), op::jmpb(RegId::ZERO, 0)],
            vec![],
        );
    }

    // logd
    {
        for i in arb_dependent_cost_values() {
            let mut instructions = setup_instructions();
            instructions.extend(vec![
                op::movi(0x13, i),
                op::logd(0x10, 0x11, RegId::ZERO, 0x13),
                op::jmpb(RegId::ZERO, 0),
            ]);
            let id = format!("contract/logd {:?}", i);
            shared_runner_builder.build().run(
                &id,
                group,
                instructions,
                script_data.clone(),
            );
        }
    }

    // mint
    {
        let contract = vec![op::mint(RegId::ONE, RegId::ZERO), op::jmpb(RegId::ZERO, 0)];
        let instructions = call_contract_once();
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/mint",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // ret contract
    {
        let contract = vec![op::ret(RegId::ONE), op::ret(RegId::ZERO)];
        let instructions = call_contract_repeat();
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/ret contract",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // retd contract
    {
        for i in arb_dependent_cost_values() {
            let contract = vec![op::movi(0x14, i), op::retd(RegId::ONE, 0x14)];
            let instructions = call_contract_repeat();
            // replace_contract_in_service(&mut service, &contract_id, contract);
            let id = format!("contract/retd contract {:?}", i);
            shared_runner_builder.build_with_new_contract(contract).run(
                &id,
                group,
                instructions,
                script_data.clone(),
            );
        }
    }

    //     run_group_ref(
    //         &mut c.benchmark_group("rvrt_contract"),
    //         "rvrt_contract",
    //         VmBench::contract(rng, op::ret(RegId::ONE)).unwrap(),
    //     );

    // TODO: Is `rvrt` even possible to bench?
    // {
    //     let contract = vec![op::rvrt(RegId::ONE)];
    //     let instructions = call_contract_repeat();
    //     replace_contract_in_service(&mut service, &contract_id, contract);
    //     run_with_service(
    //         "contract/rvrt contract",
    //         group,
    //         call_contract_repeat(),
    //         script_data.clone(),
    //         &service,
    //         contract_id,
    //         &rt,
    //         &mut rng,
    //     );
    // }

    // smo

    {
        let predicate = op::ret(RegId::ONE).to_bytes().to_vec();
        let owner = Input::predicate_owner(&predicate);
        let coin_input = Input::coin_predicate(
            Default::default(),
            owner,
            u32::MAX as Word,
            AssetId::zeroed(),
            Default::default(),
            Default::default(),
            Default::default(),
            predicate,
            vec![],
        );
        let extra_inputs = vec![coin_input];
        for i in arb_dependent_cost_values() {
            let contract = vec![
                op::gtf_args(0x15, 0x00, GTFArgs::ScriptData),
                // Offset 32 + 8 + 8 + 32
                op::addi(0x15, 0x15, 32 + 8 + 8 + 32), // target address pointer
                op::addi(0x16, 0x15, 32),              // data ppinter
                op::movi(0x17, i),                     // data length
                op::smo(0x15, 0x16, 0x17, 0x18),
                op::jmpb(RegId::ZERO, 0),
            ];
            let mut instructions = setup_instructions();
            instructions.extend(vec![
                op::movi(0x18, 1), // coins to send
                op::call(0x10, 0x12, 0x11, 0x12),
            ]);
            let mut data = script_data.clone();
            data.extend(
                Address::new([1u8; 32])
                    .iter()
                    .copied()
                    .chain(vec![2u8; i as usize]),
            );
            let id = format!("contract/smo {:?}", i);
            shared_runner_builder
                .build_with_new_contract(contract)
                .with_extra_inputs(extra_inputs.clone())
                .run(&id, group, instructions, data);
        }
    }

    // scwq

    // TODO: This is under-costed, so it runs too long and will complete before running out
    //   of gas at 100_000.
    // let size = 2620_u32; // 18bit integer maxes at 262144
    // let contract: Vec<_> = (0..100_u32)
    //     .map(|x| x * size)
    //     .map(|x| vec![op::movi(0x13, x), op::scwq(0x13, 0x29, 0x14)]) // copy range starting at $rA of size $rC
    //     .flatten()
    //     .collect();
    // let gas = 100_000;
    // let instructions = vec![
    //     op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
    //     op::addi(0x11, 0x10, ContractId::LEN.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::movi(0x12, gas),
    //     op::movi(0x14, size),
    //     op::call(0x10, RegId::ZERO, 0x11, 0x12),
    // ];
    // replace_contract_in_service(&mut service, &contract_id, contract);
    // run_with_service(
    //     "contract/scwq",
    //     group,
    //     instructions,
    //     script_data.clone(),
    //     &service,
    //     contract_id,
    //     &rt,
    //     &mut rng,
    // );

    // srw

    {
        let contract = vec![op::srw(0x13, 0x14, 0x15), op::ret(RegId::ZERO)];
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x15, 2000),
            op::call(0x10, RegId::ZERO, 0x11, 0x12),
            op::jmpb(RegId::ZERO, 0),
        ]);
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/srw",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // srwq

    // TODO: This is under-costed, so it runs too long and will complete before running out of gas
    // let size = 2620_u32;
    // let contract = (0..2620)
    //     .map(|x| x * size)
    //     .map(|x| vec![op::movi(0x13, x), op::srwq(0x14, 0x29, 0x13, 0x15)])
    //     .flatten()
    //     .collect();
    // let gas = 100_000;
    // let instructions = vec![
    //     op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
    //     op::addi(0x11, 0x10, ContractId::LEN.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::movi(0x12, gas),
    //     op::movi(0x15, size),
    //     op::call(0x10, RegId::ZERO, 0x11, 0x12),
    // ];
    // replace_contract_in_service(&mut service, &contract_id, contract);
    // run_with_service(
    //     "contract/srwq",
    //     group,
    //     instructions,
    //     script_data.clone(),
    //     &service,
    //     contract_id,
    //     &rt,
    //     &mut rng,
    // );

    // sww

    for i in 1..5 {
        let contract = vec![op::sww(RegId::ZERO, 0x29, 0x13), op::jmpb(RegId::ZERO, 0)];
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x13, i),
            op::call(0x10, RegId::ZERO, 0x11, 0x12),
        ]);
        let id = format!("contract/sww {:?}", i);
        shared_runner_builder.build_with_new_contract(contract).run(
            &id,
            group,
            instructions,
            script_data.clone(),
        );
    }

    // swwq

    // TODO: This is under-costed, so it runs too long and will complete before running out of gas
    // let size = 2620_u32;
    // // Copy value stored at $rC to the state starting at 0x13
    // let contract = (0..2620)
    //     .map(|x| x * size)
    //     .map(|x| vec![op::movi(0x13, x), op::swwq(0x13, 0x29, 0x14, 0x15)])         .flatten()
    //     .collect();
    // let gas = 100_000;
    // let value = 2000;
    // let instructions = vec![
    //     op::gtf_args(0x10, 0x00, GTFArgs::ScriptData),
    //     op::addi(0x11, 0x10, ContractId::LEN.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::addi(0x11, 0x11, WORD_SIZE.try_into().unwrap()),
    //     op::movi(0x12, gas),
    //     op::movi(0x14, value),
    //     op::movi(0x15, size),
    //     op::call(0x10, RegId::ZERO, 0x11, 0x12),
    // ];
    // replace_contract_in_service(&mut service, &contract_id, contract);
    // run_with_service(
    //     "contract/swwq",
    //     group,
    //     instructions,
    //     script_data.clone(),
    //     &service,
    //     contract_id,
    //     &rt,
    //     &mut rng,
    // );

    // time

    {
        run(
            "contract/time",
            group,
            vec![
                op::movi(0x10, 0),
                op::time(0x11, 0x10),
                op::jmpb(RegId::ZERO, 0),
            ],
            vec![],
        );
    }

    // tr
    {
        let contract = vec![op::tr(0x15, 0x14, 0x15), op::jmpb(RegId::ZERO, 0)];
        let mut instructions = setup_instructions();
        instructions.extend(vec![
            op::movi(0x13, 1 << 18 - 1),
            op::movi(0x15, 2000),
            op::movi(0x14, 1),
            op::call(0x10, 0x13, 0x15, 0x12),
            op::jmpb(RegId::ZERO, 0),
        ]);
        shared_runner_builder.build_with_new_contract(contract).run(
            "contract/tr",
            group,
            instructions,
            script_data.clone(),
        );
    }

    // tro

    // The `tro` benchmark is disabled because it would require many, many outputs, because each
    // would get spent. But it's okay because that is putting a limit of 255 outputs per transaction
    // and that protects us from an attacker exploiting a poorly priced `tro` instruction.
    // {
    //     let amount = 100;
    //
    //     let contract = vec![
    //         op::tro(RegId::ZERO, 0x15, 0x14, RegId::HP),
    //         op::ret(RegId::ZERO),
    //     ];
    //     let mut instructions = setup_instructions();
    //     instructions.extend(vec![
    //         op::movi(0x14, amount),
    //         op::movi(0x15, 1),
    //         op::movi(0x20, 32),
    //         op::aloc(0x20),
    //     ]);
    //
    //     for (i, v) in (*AssetId::zeroed()).into_iter().enumerate() {
    //         instructions.push(op::movi(0x20, v as u32));
    //         instructions.push(op::sb(RegId::HP, 0x20, i as u16));
    //     }
    //
    //     instructions.extend(vec![
    //         op::call(0x10, RegId::ZERO, 0x11, 0x12),
    //         op::jmpb(RegId::ZERO, 0),
    //     ]);
    //
    //     let predicate = op::ret(RegId::ONE).to_bytes().to_vec();
    //     let owner = Input::predicate_owner(&predicate);
    //     let coin_input = Input::coin_predicate(
    //         Default::default(),
    //         owner,
    //         1000,
    //         AssetId::zeroed(),
    //         Default::default(),
    //         Default::default(),
    //         Default::default(),
    //         predicate,
    //         vec![],
    //     );
    //     let coin_output = Output::variable(Address::zeroed(), 0, AssetId::zeroed());
    //     let extra_inputs = vec![coin_input];
    //     let extra_outputs = vec![coin_output];
    //
    //     //     replace_contract_in_service(&mut service, &contract_id, contract);
    //     //     run_with_service_with_extra_inputs(
    //     //         "contract/tro",
    //     //         group,
    //     //         instructions,
    //     //         script_data.clone(),
    //     //         &service,
    //     //         contract_id,
    //     //         &rt,
    //     //         &mut rng,
    //     //         extra_inputs,
    //     //         extra_outputs,
    //     //     );
    //     // }
    //     shared_runner_builder
    //         .build_with_new_contract(contract)
    //         .with_extra_inputs(extra_inputs)
    //         .with_extra_outputs(extra_outputs)
    //         .run(
    //             "contract/tro",
    //             group,
    //             setup_instructions(),
    //             script_data.clone(),
    //         );
    // }
}
