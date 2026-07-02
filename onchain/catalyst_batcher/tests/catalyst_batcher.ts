import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CatalystBatcher } from "../target/types/catalyst_batcher";
import { expect } from "chai";
import { PublicKey, Keypair } from "@solana/web3.js";

describe("catalyst_batcher", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CatalystBatcher as Program<CatalystBatcher>;
  const authority = provider.wallet.publicKey;

  const [configPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("global_config")],
    program.programId
  );

  const batchId = new Uint8Array(32);
  batchId[0] = 1;
  const batchHash = new Uint8Array(32);
  batchHash[0] = 0xab;

  const [receiptPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("batch_receipt"), batchId],
    program.programId
  );

  it("initializes config", async () => {
    const tx = await program.methods
      .initializeConfig(authority)
      .accounts({})
      .rpc();

    const config = await program.account.globalConfig.fetch(configPda);
    expect(config.authority.toBase58()).to.equal(authority.toBase58());
    expect(config.paused).to.equal(false);
    expect(config.batchCount.toNumber()).to.equal(0);
  });

  it("updates policy", async () => {
    const policyName = new Uint8Array(32);
    Buffer.from("adaptive").forEach((b, i) => (policyName[i] = b));

    await program.methods
      .updatePolicy(Array.from(policyName) as any)
      .accounts({ authority })
      .rpc();

    const config = await program.account.globalConfig.fetch(configPda);
    expect(Buffer.from(config.policyName).toString("utf8").replace(/\0/g, "")).to.equal("adaptive");
  });

  it("submits batch", async () => {
    await program.methods
      .submitBatch(
        Array.from(batchId) as any,
        Array.from(batchHash) as any,
        10,
        new anchor.BN(500000)
      )
      .accounts({})
      .rpc();

    const receipt = await program.account.batchReceipt.fetch(receiptPda);
    expect(receipt.txCount).to.equal(10);
    expect(receipt.totalCompute.toNumber()).to.equal(500000);
    expect(receipt.verified).to.equal(false);

    const config = await program.account.globalConfig.fetch(configPda);
    expect(config.batchCount.toNumber()).to.equal(1);
  });

  it("submits proof", async () => {
    const proofHash = new Uint8Array(32);
    proofHash[0] = 0xcd;

    await program.methods
      .submitProof(Array.from(proofHash) as any)
      .accounts({ receipt: receiptPda })
      .rpc();

    const receipt = await program.account.batchReceipt.fetch(receiptPda);
    expect(receipt.proofHash[0]).to.equal(0xcd);
  });

  it("verifies proof", async () => {
    await program.methods
      .verifyProof()
      .accounts({ receipt: receiptPda, authority })
      .rpc();

    const receipt = await program.account.batchReceipt.fetch(receiptPda);
    expect(receipt.verified).to.equal(true);
  });

  it("pause and resume", async () => {
    await program.methods.pause().accounts({ authority }).rpc();
    let config = await program.account.globalConfig.fetch(configPda);
    expect(config.paused).to.equal(true);

    await program.methods.resume().accounts({ authority }).rpc();
    config = await program.account.globalConfig.fetch(configPda);
    expect(config.paused).to.equal(false);
  });
});
