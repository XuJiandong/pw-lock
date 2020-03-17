// #include "blake2b.h"
#include "ckb_syscalls.h"
#include "common.h"
#include "protocol.h"
#include "secp256k1_helper.h"
#include "keccak256.h"
#include "bech32.h"

#define BLAKE2B_BLOCK_SIZE 32
#define BLAKE160_SIZE 20
#define PUBKEY_SIZE 65  // ETH address uncompress pub key 
#define TEMP_SIZE 32768
#define RECID_INDEX 64
/* 32 KB */
#define MAX_WITNESS_SIZE 32768
#define SCRIPT_SIZE 32768
#define SIGNATURE_SIZE 65


#define MAX_OUTPUT_LENGTH 64

#define ERROR_TOO_MANY_OUTPUT_CELLS -18
#define ERROR_OVERFLOW -13

#if (MAX_WITNESS_SIZE > TEMP_SIZE) || (SCRIPT_SIZE > TEMP_SIZE)
#error "Temp buffer is not big enough!"
#endif


static int hash_address(mol_seg_t *script_seg, unsigned char * hash){

  mol_seg_t args_seg = MolReader_Script_get_args(script_seg);
  mol_seg_t args_bytes_seg = MolReader_Bytes_raw_bytes(&args_seg);

  mol_seg_t code_hash_seg = MolReader_Script_get_code_hash(script_seg);
  mol_seg_t hash_type_seg = MolReader_Script_get_hash_type(script_seg);


  //0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8
  unsigned char block_assembler_code_hash[32] = {
      0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9, 0xfc, 0xc8, 0x8e, 0x5d, 0x4b, 0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8};
  size_t payload_len = 0;
  size_t data_len = 0;
  unsigned char payload[1024];
  unsigned char ckb_address[1024];
  unsigned char data[1024];

  unsigned char formated_ckb_address[17];
  int ret = 0;

  if (code_hash_seg.size == 0) {
    // no lock script
    data_len = 7;
    memcpy(ckb_address, "unknown", 7);
  }
  else {
    if (memcmp(code_hash_seg.ptr, block_assembler_code_hash, code_hash_seg.size) == 0) {
      //generate short ckb address
      payload[payload_len++] = 0x01;
      payload[payload_len++] = 0x00;
    }
    else {
      if (*hash_type_seg.ptr == 0x01) {
        payload[payload_len++] = 0x04;
      }
      else {
        payload[payload_len++] = 0x02;
      }
      memcpy((void *)(payload + payload_len), code_hash_seg.ptr, code_hash_seg.size);
      payload_len += code_hash_seg.size;
    }
    memcpy(payload + payload_len, args_bytes_seg.ptr, args_bytes_seg.size);
    payload_len += args_bytes_seg.size;

    ret = convert_bits(data, &data_len, 5, payload, payload_len, 8, 1);
    if(ret == 0) return -10;
    ret = bech32_encode((char *)&ckb_address, "ckt", data, data_len);
    if(ret == 0) return -11;
    data_len += 10;

    if (data_len <= 17) {
      memcpy(formated_ckb_address, ckb_address, data_len);
    }
    else {
      memcpy(formated_ckb_address, ckb_address, 7);
      memcpy(formated_ckb_address + 7, "...", 3);
      memcpy(formated_ckb_address + 10, ckb_address + (data_len - 7), 7);
      data_len = 17;
    }
  }

  SHA3_CTX sha3_ctx;
  keccak_init(&sha3_ctx);
  keccak_update(&sha3_ctx, formated_ckb_address, data_len);
  keccak_final(&sha3_ctx, hash);

  return CKB_SUCCESS;
}

static int hash_amount(uint64_t capacity, unsigned char * hash){
  // uint64_t temp = capacity;
  unsigned char amount[100];

  /* format capacity */
  int len = snprintf((char *)&amount, 100, "%.8fCKB", capacity/100000000.0);

  /* calculate keccak256 hash of amount */
  SHA3_CTX sha3_ctx;
  keccak_init(&sha3_ctx);

  keccak_update(&sha3_ctx, amount, len );

  keccak_final(&sha3_ctx, hash);
  return CKB_SUCCESS;

}

static int calculate_typed_data(unsigned char *tx_message, unsigned char * typed_data_hash){
  int ret;
  uint64_t len = 0;
  size_t index = 0;
  uint64_t input_capacities = 0;
  uint64_t output_capacities = 0;
  uint64_t tx_fee = 0;

  unsigned char script[SCRIPT_SIZE];
  mol_seg_t script_seg;

  /**
   * 
   *  hard coded hash
   */
  // typed prefix
  unsigned char TYPEDDATA_PREFIX[2] = {0x19, 0x01};
  // web3utils.sha3('CKBTransaction(bytes32 hash,string fee,string input-sum,Output[] to)Output(string address,string amount)')
  unsigned char CKBTRANSACTION_TYPEHASH[BLAKE2B_BLOCK_SIZE] = {
    0x17, 0xe4, 0x04, 0xd0, 0xcd, 0xcc, 0x43, 0x1e, 0xe6, 0xdf, 0x80, 0x7a, 0xbc, 0xcc, 0x69, 0x5d, 0x95, 0xd0, 0x38, 0xf5, 0x76, 0x47, 0xe2, 0xef, 0x92, 0xb9, 0x68, 0x66, 0xca, 0xe5, 0x9d, 0x04
  };
  // web3utils.sha3('Output(string address,string amount)')
  unsigned char OUTPUT_TYPEHASH[BLAKE2B_BLOCK_SIZE] = {
    0xef, 0xdd, 0x9a, 0xc6, 0xc9, 0x8f, 0xcb, 0xab, 0xc5, 0x2e, 0xf1, 0xd8, 0xa4, 0xd3, 0xac, 0xcd, 0x43, 0x96, 0x36, 0x2a, 0x21, 0x1c, 0xbf, 0x7a, 0x3c, 0x20, 0xc2, 0x89, 0x22, 0x08, 0x19, 0x13
  };
  // web3utils.sha3("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)")
  unsigned char DOMAIN_SEPARATOR[BLAKE2B_BLOCK_SIZE] = {
    0xec, 0x9e, 0x64, 0xcb, 0x49, 0x31, 0x37, 0x85, 0x0e, 0x3d, 0x5d, 0x47, 0x3c, 0xa1, 0x09, 0xea, 0xe1, 0x47, 0xad, 0xb8, 0xa6, 0xbf, 0x46, 0x0b, 0xf2, 0x06, 0xe9, 0x0f, 0x62, 0x64, 0x2e, 0x3f,
  };

  unsigned char address_hash[BLAKE2B_BLOCK_SIZE];
  unsigned char amount_hash[BLAKE2B_BLOCK_SIZE];
  unsigned char message[BLAKE2B_BLOCK_SIZE];

  while (1) {

    uint64_t capacity = 0;
    len = 8;
    ret = ckb_load_cell_by_field(((unsigned char *)&capacity), &len, 0, index,
                                   CKB_SOURCE_INPUT, CKB_CELL_FIELD_CAPACITY);

    if (ret == CKB_INDEX_OUT_OF_BOUND) {
      break;
    }
    if (ret != CKB_SUCCESS) {
      return ret;
    }
    if (len != 8) {
      return ERROR_SYSCALL;
    }

    if (__builtin_uaddl_overflow(input_capacities, capacity,
                                 &input_capacities))
    {
      return ERROR_OVERFLOW;
    }

    index += 1;

  }


  index = 0;

  SHA3_CTX sha3_ctx, sha3_ctx_output;
  /* to array hash */
  keccak_init(&sha3_ctx);
  while (1)
  {
    uint64_t capacity = 0;
    len = 8;
    ret = ckb_load_cell_by_field(((unsigned char *)&capacity), &len, 0, index,
                                   CKB_SOURCE_OUTPUT, CKB_CELL_FIELD_CAPACITY);

    if (ret == CKB_INDEX_OUT_OF_BOUND) {
      // return ret;
      break;
    }
    if (ret != CKB_SUCCESS) {
      return ret;
    }
    if (len != 8) {
      return ERROR_SYSCALL;
    }
    if (index >= MAX_OUTPUT_LENGTH) {
      return ERROR_TOO_MANY_OUTPUT_CELLS;
    }

    if (__builtin_uaddl_overflow(output_capacities, capacity,
                                 &output_capacities))
    {
      return ERROR_OVERFLOW;
    }

    len = SCRIPT_SIZE;
    ret = ckb_load_cell_by_field(script, &len, 0, index,
                                   CKB_SOURCE_OUTPUT, CKB_CELL_FIELD_LOCK);
    if (ret != CKB_SUCCESS) {
      return ERROR_SYSCALL;
    }
    if (len > SCRIPT_SIZE) {
      return ERROR_SCRIPT_TOO_LONG;
    }
    script_seg.ptr = (uint8_t *)script;
    script_seg.size = len;

    if (MolReader_Script_verify(&script_seg, false) != MOL_OK) {
      return ERROR_ENCODING;
    }

    hash_amount(capacity, amount_hash);

    ret = hash_address(&script_seg, address_hash);
    if(ret != CKB_SUCCESS) return ret;

    keccak_init(&sha3_ctx_output);
    keccak_update(&sha3_ctx_output, OUTPUT_TYPEHASH, 32);

    keccak_update(&sha3_ctx_output, address_hash, 32);
    keccak_update(&sha3_ctx_output, amount_hash, 32);

    keccak_final(&sha3_ctx_output, message);

    /* output hash */
    keccak_update(&sha3_ctx, message, 32);
    index += 1;
  }

  if (__builtin_usubl_overflow(input_capacities, output_capacities,
                               &tx_fee)) {
    // return ERROR_OVERFLOW;
    tx_fee = 0;
  }

  /* output array hash */
  keccak_final(&sha3_ctx, message);

  /* ckb tx value hash */
  keccak_init(&sha3_ctx);
  keccak_update(&sha3_ctx, CKBTRANSACTION_TYPEHASH, 32);
  //hash
  keccak_update(&sha3_ctx, tx_message, 32);
  // fee
  hash_amount(tx_fee, amount_hash);
  keccak_update(&sha3_ctx, amount_hash, 32);
  // input-sum
  hash_amount(input_capacities, amount_hash);
  keccak_update(&sha3_ctx, amount_hash, 32);
  // to
  keccak_update(&sha3_ctx, message, 32);

  keccak_final(&sha3_ctx, message);

  /* typed data hash */
  keccak_init(&sha3_ctx);
  keccak_update(&sha3_ctx, TYPEDDATA_PREFIX, 2);
  keccak_update(&sha3_ctx, DOMAIN_SEPARATOR, 32);
  keccak_update(&sha3_ctx, message, 32);

  keccak_final(&sha3_ctx, typed_data_hash);


  return CKB_SUCCESS;

}

static int verify_signature(unsigned char *message, unsigned char *lock_bytes, const void * lockargs){

  unsigned char temp[TEMP_SIZE];

  /* Load signature */
  secp256k1_context context;
  uint8_t secp_data[CKB_SECP256K1_DATA_SIZE];
  int ret = ckb_secp256k1_custom_verify_only_initialize(&context, secp_data);
  if (ret != 0) {
    return ret;
  }

  secp256k1_ecdsa_recoverable_signature signature;
  if (secp256k1_ecdsa_recoverable_signature_parse_compact(
          &context, &signature, lock_bytes, lock_bytes[RECID_INDEX]) == 0) {
    return ERROR_SECP_PARSE_SIGNATURE;
  }

  /* Recover pubkey */
  secp256k1_pubkey pubkey;
  if (secp256k1_ecdsa_recover(&context, &pubkey, &signature, message) != 1) {
    return ERROR_SECP_RECOVER_PUBKEY;
  }

  /* Check pubkey hash */
  size_t pubkey_size = PUBKEY_SIZE;
  if (secp256k1_ec_pubkey_serialize(&context, temp, &pubkey_size, &pubkey,
                                    SECP256K1_EC_UNCOMPRESSED) != 1) {
    return ERROR_SECP_SERIALIZE_PUBKEY;
  }

  SHA3_CTX sha3_ctx;
  keccak_init(&sha3_ctx);
  keccak_update(&sha3_ctx, &temp[1], pubkey_size - 1);
  keccak_final(&sha3_ctx, temp);

  if (memcmp(lockargs, &temp[12], BLAKE160_SIZE) != 0) {
    return ERROR_PUBKEY_BLAKE160_HASH;
  }

  return CKB_SUCCESS;
}

/*
 * Arguments:
 * pubkey blake160 hash, blake2b hash of pubkey first 20 bytes, used to
 * shield the real pubkey.
 *
 * Witness:
 * WitnessArgs with a signature in lock field used to present ownership.
 */
int main() {
  int ret;
  uint64_t len = 0;
  unsigned char temp[TEMP_SIZE];
  unsigned char lock_bytes[SIGNATURE_SIZE];

  /* Load args */
  unsigned char script[SCRIPT_SIZE];
  len = SCRIPT_SIZE;
  ret = ckb_load_script(script, &len, 0);
  if (ret != CKB_SUCCESS) {
    return ERROR_SYSCALL;
  }
  if (len > SCRIPT_SIZE) {
    return ERROR_SCRIPT_TOO_LONG;
  }
  mol_seg_t script_seg;
  script_seg.ptr = (uint8_t *)script;
  script_seg.size = len;

  if (MolReader_Script_verify(&script_seg, false) != MOL_OK) {
    return ERROR_ENCODING;
  }

  mol_seg_t args_seg = MolReader_Script_get_args(&script_seg);
  mol_seg_t args_bytes_seg = MolReader_Bytes_raw_bytes(&args_seg);
  if (args_bytes_seg.size != BLAKE160_SIZE) {
    return ERROR_ARGUMENTS_LEN;
  }

  /* Load witness of first input */
  uint64_t witness_len = MAX_WITNESS_SIZE;
  ret = ckb_load_witness(temp, &witness_len, 0, 0, CKB_SOURCE_GROUP_INPUT);
  if (ret != CKB_SUCCESS) {
    return ERROR_SYSCALL;
  }

  if (witness_len > MAX_WITNESS_SIZE) {
    return ERROR_WITNESS_SIZE;
  }

  /* load signature */
  mol_seg_t lock_bytes_seg;
  ret = extract_witness_lock(temp, witness_len, &lock_bytes_seg);
  if (ret != 0) {
    return ERROR_ENCODING;
  }

  if (lock_bytes_seg.size != SIGNATURE_SIZE) {
    return ERROR_ARGUMENTS_LEN;
  }
  memcpy(lock_bytes, lock_bytes_seg.ptr, lock_bytes_seg.size);

  /* Load tx hash */
  unsigned char tx_hash[BLAKE2B_BLOCK_SIZE];
  len = BLAKE2B_BLOCK_SIZE;
  ret = ckb_load_tx_hash(tx_hash, &len, 0);
  if (ret != CKB_SUCCESS) {
    return ret;
  }
  if (len != BLAKE2B_BLOCK_SIZE) {
    return ERROR_SYSCALL;
  }

  /* Prepare sign message */
  unsigned char message[BLAKE2B_BLOCK_SIZE];
  // blake2b_state blake2b_ctx;
  // blake2b_init(&blake2b_ctx, BLAKE2B_BLOCK_SIZE);
  // blake2b_update(&blake2b_ctx, tx_hash, BLAKE2B_BLOCK_SIZE);
  SHA3_CTX sha3_ctx;
  keccak_init(&sha3_ctx);
  keccak_update(&sha3_ctx, tx_hash, BLAKE2B_BLOCK_SIZE);


  /* Clear lock field to zero, then digest the first witness */
  memset((void *)lock_bytes_seg.ptr, 0, lock_bytes_seg.size);
  // blake2b_update(&blake2b_ctx, (char *)&witness_len, sizeof(uint64_t));
  // blake2b_update(&blake2b_ctx, temp, witness_len);
  keccak_update(&sha3_ctx, (unsigned char *)&witness_len, sizeof(uint64_t));
  keccak_update(&sha3_ctx, temp, witness_len);

  /* Digest same group witnesses */
  size_t i = 1;
  while (1) {
    len = MAX_WITNESS_SIZE;
    ret = ckb_load_witness(temp, &len, 0, i, CKB_SOURCE_GROUP_INPUT);
    if (ret == CKB_INDEX_OUT_OF_BOUND) {
      break;
    }
    if (ret != CKB_SUCCESS) {
      return ERROR_SYSCALL;
    }
    if (len > MAX_WITNESS_SIZE) {
      return ERROR_WITNESS_SIZE;
    }
    // blake2b_update(&blake2b_ctx, (char *)&len, sizeof(uint64_t));
    // blake2b_update(&blake2b_ctx, temp, len);
    keccak_update(&sha3_ctx, (unsigned char *)&len, sizeof(uint64_t));
    keccak_update(&sha3_ctx, temp, len);
    i += 1;
  }
  /* Digest witnesses that not covered by inputs */
  i = calculate_inputs_len();
  while (1) {
    len = MAX_WITNESS_SIZE;
    ret = ckb_load_witness(temp, &len, 0, i, CKB_SOURCE_INPUT);
    if (ret == CKB_INDEX_OUT_OF_BOUND) {
      break;
    }
    if (ret != CKB_SUCCESS) {
      return ERROR_SYSCALL;
    }
    if (len > MAX_WITNESS_SIZE) {
      return ERROR_WITNESS_SIZE;
    }
    // blake2b_update(&blake2b_ctx, (char *)&len, sizeof(uint64_t));
    // blake2b_update(&blake2b_ctx, temp, len);
    keccak_update(&sha3_ctx, (unsigned char *)&len, sizeof(uint64_t));
    keccak_update(&sha3_ctx, temp, len);

    i += 1;
  }
  keccak_final(&sha3_ctx, message);

  /* personal hash */
  keccak_init(&sha3_ctx);
  unsigned char eth_prefix[28]= {
0x19, 0x45, 0x74, 0x68, 0x65, 0x72, 0x65, 0x75, 0x6d, 0x20, 0x53, 0x69 ,0x67, 0x6e , 
0x65 , 0x64 , 0x20 , 0x4d , 0x65 , 0x73 , 0x73, 0x61 , 0x67 , 0x65 , 0x3a , 0x0a , 0x33 , 0x32
  };
  keccak_update(&sha3_ctx, eth_prefix, 28);
  keccak_update(&sha3_ctx, message, 32);
  keccak_final(&sha3_ctx, message);

  ret = verify_signature(message, lock_bytes, args_bytes_seg.ptr);
  if(ret == CKB_SUCCESS){
    return CKB_SUCCESS;
  }

  /* Calculate Typed Data hash */
  ret = calculate_typed_data(message, message);
  if(ret != CKB_SUCCESS){
    return ret;
  }

  return verify_signature(message, lock_bytes, args_bytes_seg.ptr);
}
