#![allow(dead_code)]

use super::{
    eth160, get_current_chain_id, is_compressed, pubkey_compressed, pubkey_uncompressed,
    ripemd_sha, sign_tx_by_input_group_keccak256, sign_tx_keccak256, sign_tx_keccak256_with_flag,
    DummyDataLoader, CHAIN_ID_BTC, CHAIN_ID_DOGE, CHAIN_ID_EOS, CHAIN_ID_ETH, CHAIN_ID_TRON,
    KECCAK256_ALL_ACPL_BIN, MAX_CYCLES, SECP256K1_DATA_BIN,
};
use ckb_crypto::secp::{Generator, Privkey};
use ckb_error::assert_error_eq;
use ckb_script::{ScriptError, TransactionScriptsVerifier};
use ckb_types::{
    bytes::Bytes,
    core::{
        cell::{CellMetaBuilder, ResolvedTransaction},
        Capacity, DepType, ScriptHashType, TransactionBuilder, TransactionView,
    },
    packed::{CellDep, CellInput, CellOutput, OutPoint, Script, WitnessArgs, WitnessArgsBuilder},
    prelude::*,
    H256,
};
use rand::{thread_rng, Rng, SeedableRng};

use sha3::{Digest, Keccak256};

const ERROR_ENCODING: i8 = -2;
// const ERROR_LENGTH_NOT_ENOUGH: i8 = -3;
const ERROR_PUBKEY_BLAKE160_HASH: i8 = -31;

fn gen_tx(dummy: &mut DummyDataLoader, lock_args: Bytes) -> TransactionView {
    let mut rng = thread_rng();
    gen_tx_with_grouped_args(dummy, vec![(lock_args, 1)], &mut rng)
}

fn gen_tx_with_grouped_args<R: Rng>(
    dummy: &mut DummyDataLoader,
    grouped_args: Vec<(Bytes, usize)>,
    rng: &mut R,
) -> TransactionView {
    // setup sighash_all dep
    let sighash_all_out_point = {
        let contract_tx_hash = {
            let mut buf = [0u8; 32];
            rng.fill(&mut buf);
            buf.pack()
        };
        OutPoint::new(contract_tx_hash.clone(), 0)
    };
    // dep contract code
    let sighash_all_cell = CellOutput::new_builder()
        .capacity(
            Capacity::bytes(KECCAK256_ALL_ACPL_BIN.len())
                .expect("script capacity")
                .pack(),
        )
        .build();
    let sighash_all_cell_data_hash = CellOutput::calc_data_hash(&KECCAK256_ALL_ACPL_BIN);
    dummy.cells.insert(
        sighash_all_out_point.clone(),
        (sighash_all_cell, KECCAK256_ALL_ACPL_BIN.clone()),
    );
    // setup secp256k1_data dep
    let secp256k1_data_out_point = {
        let tx_hash = {
            let mut buf = [0u8; 32];
            rng.fill(&mut buf);
            buf.pack()
        };
        OutPoint::new(tx_hash, 0)
    };
    let secp256k1_data_cell = CellOutput::new_builder()
        .capacity(
            Capacity::bytes(SECP256K1_DATA_BIN.len())
                .expect("data capacity")
                .pack(),
        )
        .build();
    dummy.cells.insert(
        secp256k1_data_out_point.clone(),
        (secp256k1_data_cell, SECP256K1_DATA_BIN.clone()),
    );
    // setup default tx builder

    let block_assembler_code_hash: [u8; 32] = [
        0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1,
        0xb9, 0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3,
        0xcc, 0xe8,
    ];
    let lock_script = Script::new_builder()
        .code_hash(block_assembler_code_hash.pack())
        .args([0u8; 33].pack())
        .hash_type(ScriptHashType::Type.into())
        .build();
    let dummy_capacity = Capacity::shannons(42);
    let mut tx_builder = TransactionBuilder::default()
        .cell_dep(
            CellDep::new_builder()
                .out_point(sighash_all_out_point)
                .dep_type(DepType::Code.into())
                .build(),
        )
        .cell_dep(
            CellDep::new_builder()
                .out_point(secp256k1_data_out_point)
                .dep_type(DepType::Code.into())
                .build(),
        )
        .output(
            CellOutput::new_builder()
                .capacity(dummy_capacity.pack())
                .lock(lock_script)
                .build(),
        )
        .output_data(Bytes::new().pack());

    for (args, inputs_size) in grouped_args {
        // setup dummy input unlock script
        for _ in 0..inputs_size {
            let previous_tx_hash = {
                let mut buf = [0u8; 32];
                rng.fill(&mut buf);
                buf.pack()
            };
            let previous_out_point = OutPoint::new(previous_tx_hash, 0);
            let script = Script::new_builder()
                .args(args.pack())
                .code_hash(sighash_all_cell_data_hash.clone())
                .hash_type(ScriptHashType::Data.into())
                .build();
            let previous_output_cell = CellOutput::new_builder()
                .capacity(dummy_capacity.pack())
                .lock(script)
                .build();
            dummy.cells.insert(
                previous_out_point.clone(),
                (previous_output_cell.clone(), Bytes::new()),
            );
            let mut random_extra_witness = [0u8; 32];
            rng.fill(&mut random_extra_witness);
            let witness_args = WitnessArgsBuilder::default()
                .extra(Bytes::from(random_extra_witness.to_vec()).pack())
                .build();
            tx_builder = tx_builder
                .input(CellInput::new(previous_out_point, 0))
                .witness(witness_args.as_bytes().pack());
        }
    }

    tx_builder.build()
}

fn sign_tx_hash(tx: TransactionView, key: &Privkey, tx_hash: &[u8]) -> TransactionView {
    // calculate message
    let mut hasher = Keccak256::default();
    let mut message = [0u8; 32];
    hasher.input(tx_hash);
    message.copy_from_slice(&hasher.result()[0..32]);
    let message = H256::from(message);

    let sig = key.sign_recoverable(&message).expect("sign");
    let witness_args = WitnessArgsBuilder::default()
        .lock(Bytes::from(sig.serialize()).pack())
        .build();
    tx.as_advanced_builder()
        .set_witnesses(vec![witness_args.as_bytes().pack()])
        .build()
}

fn build_resolved_tx(data_loader: &DummyDataLoader, tx: &TransactionView) -> ResolvedTransaction {
    let resolved_cell_deps = tx
        .cell_deps()
        .into_iter()
        .map(|dep| {
            let deps_out_point = dep.clone();
            let (dep_output, dep_data) =
                data_loader.cells.get(&deps_out_point.out_point()).unwrap();
            CellMetaBuilder::from_cell_output(dep_output.to_owned(), dep_data.to_owned())
                .out_point(deps_out_point.out_point().clone())
                .build()
        })
        .collect();

    let mut resolved_inputs = Vec::new();
    for i in 0..tx.inputs().len() {
        let previous_out_point = tx.inputs().get(i).unwrap().previous_output();
        let (input_output, input_data) = data_loader.cells.get(&previous_out_point).unwrap();
        resolved_inputs.push(
            CellMetaBuilder::from_cell_output(input_output.to_owned(), input_data.to_owned())
                .out_point(previous_out_point)
                .build(),
        );
    }

    ResolvedTransaction {
        transaction: tx.clone(),
        resolved_cell_deps,
        resolved_inputs,
        resolved_dep_groups: vec![],
    }
}

#[test]
fn test_keccak_all_unlock() {
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let tx = sign_tx_keccak256(&mut data_loader, tx, &privkey);
    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    let _cycle = verify_result.expect("pass verification");
    // assert_eq!(cycle > 10000000, true);
}

#[test]
fn test_keccak_all_unlock_btc_with_fix_sk() {
    //only run for BTC
    if get_current_chain_id() == CHAIN_ID_BTC {
        let mut data_loader = DummyDataLoader::new();
        let mut sk = vec![0u8; 32];
        //WIF format: L5EZftvrYaSudiozVRzTqLcHLNDoVn7H5HSfM9BAN6tMJX8oTWz6
        faster_hex::hex_decode(
            b"ef235aacf90d9f4aadd8c92e4b2562e1d9eb97f0df9ba3b508258739cb013db2",
            &mut sk,
        )
        .ok()
        .unwrap();
        let privkey = Privkey::from_slice(&sk);
        let pubkey = privkey.pubkey().expect("pubkey");
        let pubkey_hash = {
            //only for compressed
            let pubkey = pubkey_compressed(&pubkey);
            ripemd_sha(&pubkey)
        };

        //fix output rng
        let mut rng = rand_chacha::ChaChaRng::seed_from_u64(1);
        let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 1)], &mut rng);
        let tx = sign_tx_keccak256(&mut data_loader, tx, &privkey);
        let resolved_tx = build_resolved_tx(&data_loader, &tx);
        let verify_result =
            TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
        verify_result.expect("pass verification");
    }
}

#[test]
fn test_sighash_all_with_extra_witness_unlock() {
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let extract_witness = vec![1, 2, 3, 4];
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![WitnessArgs::new_builder()
            .extra(Bytes::from(extract_witness).pack())
            .build()
            .as_bytes()
            .pack()])
        .build();
    {
        let tx = sign_tx_keccak256(&mut data_loader, tx.clone(), &privkey);
        let resolved_tx = build_resolved_tx(&data_loader, &tx);
        let verify_result =
            TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
        verify_result.expect("pass verification");
    }
    {
        let tx = sign_tx_keccak256(&mut data_loader, tx, &privkey);
        let wrong_witness = tx
            .witnesses()
            .get(0)
            .map(|w| {
                WitnessArgs::new_unchecked(w.unpack())
                    .as_builder()
                    .extra(Bytes::from(vec![0]).pack())
                    .build()
            })
            .unwrap();
        let tx = tx
            .as_advanced_builder()
            .set_witnesses(vec![wrong_witness.as_bytes().pack()])
            .build();
        let resolved_tx = build_resolved_tx(&data_loader, &tx);
        let verify_result =
            TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
        assert_error_eq!(
            verify_result.unwrap_err(),
            ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
        );
    }
}

#[test]
fn test_sighash_all_with_grouped_inputs_unlock() {
    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 2)], &mut rng);
    {
        let tx = sign_tx_keccak256(&mut data_loader, tx.clone(), &privkey);
        let resolved_tx = build_resolved_tx(&data_loader, &tx);
        let verify_result =
            TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
        verify_result.expect("pass verification");
    }
    {
        let tx = sign_tx_keccak256(&mut data_loader, tx.clone(), &privkey);
        let wrong_witness = tx
            .witnesses()
            .get(1)
            .map(|w| {
                WitnessArgs::new_unchecked(w.unpack())
                    .as_builder()
                    .extra(Bytes::from(vec![0]).pack())
                    .build()
            })
            .unwrap();
        let tx = tx
            .as_advanced_builder()
            .set_witnesses(vec![
                tx.witnesses().get(0).unwrap(),
                wrong_witness.as_bytes().pack(),
            ])
            .build();
        let resolved_tx = build_resolved_tx(&data_loader, &tx);
        let verify_result =
            TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
        assert_error_eq!(
            verify_result.unwrap_err(),
            ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
        );
    }
}

#[test]
fn test_sighash_all_with_2_different_inputs_unlock() {
    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    // key1
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };
    // key2
    let privkey2 = Generator::random_privkey();
    let pubkey2 = privkey2.pubkey().expect("pubkey");
    let pubkey_hash2 =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey2 = if is_compressed() {
                pubkey_compressed(&pubkey2)
            } else {
                pubkey_uncompressed(&pubkey2)
            };
            ripemd_sha(&pubkey2)
        } else {
            eth160(pubkey2)
        };

    // sign with 2 keys
    let tx = gen_tx_with_grouped_args(
        &mut data_loader,
        vec![(pubkey_hash, 2), (pubkey_hash2, 2)],
        &mut rng,
    );
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 2);
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey2, 2, 2);

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    verify_result.expect("pass verification");
}

#[test]
fn test_signing_with_wrong_key() {
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let wrong_privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let tx = sign_tx_keccak256(&mut data_loader, tx, &wrong_privkey);
    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_signing_wrong_tx_hash() {
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let tx = {
        let mut rand_tx_hash = [0u8; 32];
        let mut rng = thread_rng();
        rng.fill(&mut rand_tx_hash);
        sign_tx_hash(tx, &privkey, &rand_tx_hash[..])
    };
    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_super_long_witness() {
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let tx_hash = tx.hash();

    let mut buffer: Vec<u8> = vec![];
    buffer.resize(40000, 1);
    let super_long_message = Bytes::from(&buffer[..]);

    // let mut blake2b = ckb_hash::new_blake2b();
    let mut hasher = Keccak256::default();
    let mut message = [0u8; 32];
    // blake2b.update(&tx_hash.raw_data());
    // blake2b.update(&super_long_message[..]);
    // blake2b.finalize(&mut message);
    hasher.input(&tx_hash.raw_data());
    hasher.input(&super_long_message[..]);
    message.copy_from_slice(&hasher.result()[0..32]);
    let message = H256::from(message);
    let sig = privkey.sign_recoverable(&message).expect("sign");
    let witness = WitnessArgs::new_builder()
        .lock(Bytes::from(sig.serialize()).pack())
        .extra(super_long_message.pack())
        .build();
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![witness.as_bytes().pack()])
        .build();

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[ignore = "for different chain wallet signature, consumed cycles are different"]
#[test]
fn test_sighash_all_2_in_2_out_cycles() {
    const CONSUME_CYCLES: u64 = 3600353;

    let mut data_loader = DummyDataLoader::new();
    let mut generator = Generator::non_crypto_safe_prng(42);
    let mut rng = thread_rng();

    // key1
    let privkey = generator.gen_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };
    // key2
    let privkey2 = generator.gen_privkey();
    let pubkey2 = privkey2.pubkey().expect("pubkey");
    let pubkey_hash2 =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey2 = if is_compressed() {
                pubkey_compressed(&pubkey2)
            } else {
                pubkey_uncompressed(&pubkey2)
            };
            ripemd_sha(&pubkey2)
        } else {
            eth160(pubkey2)
        };

    // sign with 2 keys
    let tx = gen_tx_with_grouped_args(
        &mut data_loader,
        vec![(pubkey_hash, 1), (pubkey_hash2, 1)],
        &mut rng,
    );
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 1);
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey2, 1, 1);

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    let cycles = verify_result.expect("pass verification");
    assert_eq!(CONSUME_CYCLES, cycles)
}

#[test]
fn test_sighash_all_witness_append_junk_data() {
    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    // sign with 2 keys
    let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 2)], &mut rng);
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 2);
    let mut witnesses: Vec<_> = Unpack::<Vec<_>>::unpack(&tx.witnesses());
    // append junk data to first witness
    let mut witness = Vec::new();
    witness.resize(witnesses[0].len(), 0);
    witness.copy_from_slice(&witnesses[0]);
    witness.push(0);
    witnesses[0] = witness.into();

    let tx = tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect())
        .build();

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_sighash_all_witness_args_ambiguity() {
    // This test case build tx with WitnessArgs(lock, data, "")
    // and try unlock with WitnessArgs(lock, "", data)
    //
    // this case will fail if contract use a naive function to digest witness.

    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 2)], &mut rng);
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 2);
    let witnesses: Vec<_> = Unpack::<Vec<_>>::unpack(&tx.witnesses());
    // move extra data to type_
    let witnesses: Vec<_> = witnesses
        .into_iter()
        .map(|witness| {
            let witness = WitnessArgs::new_unchecked(witness);
            let data = witness.extra().clone();
            witness
                .as_builder()
                .extra(Bytes::new().pack())
                .type_(data)
                .build()
        })
        .collect();

    let tx = tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.as_bytes().pack()).collect())
        .build();

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_sighash_all_witnesses_ambiguity() {
    // This test case sign tx with [witness1, "", witness2]
    // and try unlock with [witness1, witness2, ""]
    //
    // this case will fail if contract use a naive function to digest witness.

    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 3)], &mut rng);
    let witness = Unpack::<Vec<_>>::unpack(&tx.witnesses()).remove(0);
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![
            witness.pack(),
            Bytes::new().pack(),
            Bytes::from(vec![42]).pack(),
        ])
        .build();
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 3);

    // exchange witness position
    let witness = Unpack::<Vec<_>>::unpack(&tx.witnesses()).remove(0);
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![
            witness.pack(),
            Bytes::from(vec![42]).pack(),
            Bytes::new().pack(),
        ])
        .build();

    assert_eq!(tx.witnesses().len(), tx.inputs().len());
    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_sighash_all_cover_extra_witnesses() {
    let mut rng = thread_rng();
    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };

    let tx = gen_tx_with_grouped_args(&mut data_loader, vec![(pubkey_hash, 2)], &mut rng);
    let witness = Unpack::<Vec<_>>::unpack(&tx.witnesses()).remove(0);
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![
            witness.pack(),
            Bytes::from(vec![42]).pack(),
            Bytes::new().pack(),
        ])
        .build();
    let tx = sign_tx_by_input_group_keccak256(&mut data_loader, tx, &privkey, 0, 3);
    assert!(tx.witnesses().len() > tx.inputs().len());

    // change last witness
    let mut witnesses = Unpack::<Vec<_>>::unpack(&tx.witnesses());
    let tx = tx
        .as_advanced_builder()
        .set_witnesses(vec![
            witnesses.remove(0).pack(),
            witnesses.remove(1).pack(),
            Bytes::from(vec![0]).pack(),
        ])
        .build();

    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(60000000);
    assert_error_eq!(
        verify_result.unwrap_err(),
        ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
    );
}

#[test]
fn test_sighash_all_no_chain_flag_in_witness() {
    // this test case set only 65-byte witness, and no chain flag
    // for ethereume, in order to compatible with old signature, it will pass verification
    // for eos/tron, the test case will failed

    let mut data_loader = DummyDataLoader::new();
    let privkey = Generator::random_privkey();
    let pubkey = privkey.pubkey().expect("pubkey");
    let pubkey_hash =
        if get_current_chain_id() == CHAIN_ID_BTC || get_current_chain_id() == CHAIN_ID_DOGE {
            let pubkey = if is_compressed() {
                pubkey_compressed(&pubkey)
            } else {
                pubkey_uncompressed(&pubkey)
            };
            ripemd_sha(&pubkey)
        } else {
            eth160(pubkey)
        };
    let tx = gen_tx(&mut data_loader, pubkey_hash);
    let tx = sign_tx_keccak256_with_flag(&mut data_loader, tx, &privkey, false);
    let resolved_tx = build_resolved_tx(&data_loader, &tx);
    let verify_result =
        TransactionScriptsVerifier::new(&resolved_tx, &data_loader).verify(MAX_CYCLES);

    let chain_id = get_current_chain_id();

    println!("chain_id: {}", chain_id);

    match chain_id {
        CHAIN_ID_ETH => {
            verify_result.expect("pass verification");
        }
        CHAIN_ID_EOS | CHAIN_ID_TRON => {
            assert_error_eq!(
                verify_result.unwrap_err(),
                ScriptError::ValidationFailure(ERROR_PUBKEY_BLAKE160_HASH),
            );
        }

        _val => {}
    }
}
