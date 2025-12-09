# MIKO Protocol On-Chain Programs

This repository contains the verified source code for the core on-chain programs of the **MIKO Protocol**. These smart contracts serve as the foundation of trust and security for the MIKO ecosystem, working in collaboration with off-chain automation services.

## 🏛️ Core Programs

The repository includes the following two main programs:

### 1. Absolute Vault
*   **Role:** Central Treasury & Authority Management
*   **Description:** A secure smart contract that acts as the protocol's central vault. It is responsible for:
    *   Securely storing transaction fees harvested by the Keeper Bot.
    *   Managing transferred authorities.
    *   Recording all fund movements on-chain for complete transparency.

### 2. Smart Dial
*   **Role:** Weekly Reward Configuration
*   **Description:** A crucial configuration program that stores the address of the weekly reward token selected by the MIKO AI Agent.
    *   The Keeper Bot queries this contract to determine which token to swap and distribute to holders each week.
    *   Ensures the reward distribution rule is verifiable on-chain.

## ⚙️ Architecture Overview

The MIKO Protocol operates as a hybrid system combining on-chain immutability with off-chain flexibility:

1.  **MIKO Token (Token-2022):** Imposes a fixed 6% transfer fee.
2.  **Keeper Bot (Off-Chain):** Monitors the fee threshold (0.05% supply), harvests fees, and executes the swap & distribution logic based on the **Smart Dial** configuration.
3.  **On-Chain Verification:** All assets are stored in the **Absolute Vault**, ensuring that funds are only moved according to the pre-defined protocol rules.

For more technical details, please refer to our [Official Documentation](https://docs.mikoprotocol.com).

## 📜 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---
*Powered by MIKO Protocol*
