/**
 * The file perform TRON wallet signature verification.
 *
 * The TRON wallet signature is generated by window.tronWeb.trx.sign().
 * tronWeb API is refer to
 * https://developers.tron.network/docs/tronlink-integration#signature
 *
 *
 * @message: transaction message digest
 * @eth_address: keccak256 hash of pubkey last 20 bytes, used to shield the real
 * pubkey.
 * @lock_bytes: transaction signature in witness.lock
 *
 */
int verify_secp256k1_keccak_tron_sighash_all(unsigned char* message,
                                             unsigned char* eth_address,
                                             unsigned char* lock_bytes) {
  SHA3_CTX sha3_ctx;
  keccak_init(&sha3_ctx);
  /* personal hash, ethereum prefix  \x19TRON Signed Message:\n32  */
  unsigned char tron_prefix[24] = {
      0x19, 0x54, 0x52, 0x4f, 0x4e, 0x20, 0x53, 0x69, 0x67, 0x6e, 0x65, 0x64,
      0x20, 0x4d, 0x65, 0x73, 0x73, 0x61, 0x67, 0x65, 0x3a, 0x0a, 0x33, 0x32};
  keccak_update(&sha3_ctx, tron_prefix, 24);
  keccak_update(&sha3_ctx, message, 32);
  keccak_final(&sha3_ctx, message);

  /* verify signature with peronsal hash */
  return verify_signature(message, lock_bytes, eth_address);
}
