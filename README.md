# Using Saya: Essential Steps

To effectively use Saya, there are a few critical steps to follow:

## 1. Pathfinder
Pathfinder plays crucial role as its providing all neccesary information about blocks, and its plays crucial role 

## 2. Piltover Contract
### Piltover deployment:
### Piltover class hash: `0x00dfe01a79ece929d976d5b8d58f7d3e368765f7a33efe2b6fa7efa258e340ae`
> Note: Current piltover implementation can be found here: [Piltover](https://github.com/chudkowsky/piltover/tree/feat/fact-registry)
#### Step 1: Deployment with Constructor Arguments
Deploy the contract with the following constructor arguments:
- **Owner Account Address**: Specify the account address of the owner.
- **Initial State Root**: Provide the initial state root.
- **Initial Block Number**: Set the initial block number.
- **Initial Block Hash**: Set the initial block hash.
---
#### Step 2: Set Program Info
Invoke the `set_program_info` function with the following calldata:
- **Program Hash**: `0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07`
> Note: The program hash value is constant, comes from layout bridge and should not be changed.
---
#### Step 3: Set Fact Registry Address
Invoke the `set_facts_registry` function with the following argument:
- **Fact Registry Contract Address**: Provide the contract address of the fact registry.
> Note: Here is current fact registry addres used by atlantic:  `0x4ce7851f00b6c3289674841fd7a1b96b6fd41ed1edc248faccd672c26371b8c` but it might change in future.
---
##### Summary
1. Deploy the contract with the required constructor arguments.
2. Set the program information using the `set_program_info` function with the fixed program hash.
3. Set the fact registry address using the `set_facts_registry` function.

## 3. API Keys for Atlantic
To generate an api key, you must first register here  https://staging.dashboard.herodotus.dev/
And by clicking on your username you have the option to copy the api key. 

## 4. Starnet Account
Prepare your account address and private key, as these are essential for sending transactions and interacting with the Piltover contract
