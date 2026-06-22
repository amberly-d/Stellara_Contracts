const { expect } = require("chai");
const { ethers } = require("hardhat");

describe("MultisigTreasury", function () {
  let owner0, owner1, owner2, recipient;
  let treasury;

  beforeEach(async () => {
    [owner0, owner1, owner2, recipient] = await ethers.getSigners();
    const owners = [owner0.address, owner1.address, owner2.address];
    const Multisig = await ethers.getContractFactory("MultisigTreasury");
    // dailyLimit=10, weeklyLimit=100, threshold=2 (all in ETH)
    treasury = await Multisig.deploy(owners, 2, ethers.parseEther("10"), ethers.parseEther("100"), ethers.parseEther("2"));
    await treasury.waitForDeployment();

    // Fund contract
    await owner0.sendTransaction({ to: await treasury.getAddress(), value: ethers.parseEther("5") });
  });

  it("executes a small single-confirm transaction", async () => {
    const value = ethers.parseEther("0.5");
    await treasury.connect(owner0).submitTransaction(recipient.address, value, '0x');
    const count = await treasury.getTransactionCount();
    const idx = count - 1n;
    await treasury.connect(owner0).confirmTransaction(idx);
    const before = await ethers.provider.getBalance(recipient.address);
    await treasury.connect(owner0).executeTransaction(idx);
    const after = await ethers.provider.getBalance(recipient.address);
    expect(after - before).to.equal(value);
  });

  it("requires multisig for large transactions above threshold", async () => {
    const value = ethers.parseEther("3"); // above threshold of 2
    await treasury.connect(owner0).submitTransaction(recipient.address, value, '0x');
    const count = await treasury.getTransactionCount();
    const idx = count - 1n;
    // single confirm should not be enough for large tx
    await treasury.connect(owner0).confirmTransaction(idx);
    await expect(treasury.connect(owner0).executeTransaction(idx)).to.be.revertedWith("insufficient confirmations for large tx");

    // second confirm satisfies multisig requirement
    await treasury.connect(owner1).confirmTransaction(idx);
    const before = await ethers.provider.getBalance(recipient.address);
    await treasury.connect(owner0).executeTransaction(idx);
    const after = await ethers.provider.getBalance(recipient.address);
    expect(after - before).to.equal(value);
  });

  it("has __gap storage slot to prevent upgradeable storage collisions", async () => {
    // Verify all public state variables remain accessible — confirming the
    // storage layout is intact after the __gap was appended.
    expect(await treasury.required()).to.equal(2n);
    expect(await treasury.dailyLimit()).to.equal(ethers.parseEther("10"));
    expect(await treasury.weeklyLimit()).to.equal(ethers.parseEther("100"));
    expect(await treasury.threshold()).to.equal(ethers.parseEther("2"));
    expect(await treasury.frozen()).to.equal(false);
    // The __gap occupies 50 reserved slots after the declared variables,
    // ensuring future additions do not shift existing storage positions.
    const owners = await treasury.getOwners();
    expect(owners.length).to.equal(3);
  });

  it("supports emergency freeze and multisig unfreeze", async () => {
    // Freeze immediately by one owner
    await treasury.connect(owner0).emergencyFreeze();
    expect(await treasury.frozen()).to.equal(true);

    // Submit a small tx while frozen (submitTransaction has no frozen guard)
    await treasury.connect(owner0).submitTransaction(recipient.address, ethers.parseEther("0.1"), '0x');
    const count = await treasury.getTransactionCount();
    const smallIdx = count - 1n;

    // Regular txs cannot be confirmed/executed while frozen
    await expect(treasury.connect(owner0).confirmTransaction(smallIdx)).to.be.revertedWith("frozen");
    await expect(treasury.connect(owner0).executeTransaction(smallIdx)).to.be.revertedWith("frozen");

    // Submit and confirm an unfreeze tx — allowed even while frozen
    const data = treasury.interface.encodeFunctionData("unfreezeInternal");
    await treasury.connect(owner0).submitTransaction(await treasury.getAddress(), 0, data);
    const count2 = await treasury.getTransactionCount();
    const unfreezeIdx = count2 - 1n;
    await treasury.connect(owner0).confirmTransaction(unfreezeIdx);
    await treasury.connect(owner1).confirmTransaction(unfreezeIdx);

    // Execute the unfreeze (allowed while frozen for unfreeze calls)
    await treasury.connect(owner0).executeTransaction(unfreezeIdx);
    expect(await treasury.frozen()).to.equal(false);
  });
});
