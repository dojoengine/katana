Feature: Generic Starknet Deployment Skills
  As a Starknet developer
  I want to deploy ANY contract reproducibly
  So that I can use these skills in any project

  Background:
    Given sncast CLI is installed and accessible
    And scarb is installed
    And a valid snfoundry.toml exists with network profiles

  # ==========================================================================
  # Account Management (Generic)
  # ==========================================================================

  @account @generic
  Scenario: Create a new account with specified type
    Given I am using the "<profile>" profile
    When I request to create an account named "<account_name>" of type "<account_type>"
    Then a new account should be created
    And the account address should be displayed
    And the account should be saved to the accounts file

    Examples:
      | profile | account_name | account_type |
      | devnet  | deployer-1   | oz           |
      | devnet  | deployer-2   | argent       |
      | sepolia | prod-deployer| oz           |

  @account @generic @devnet-predeployed
  Scenario: Use predeployed devnet account without setup
    Given starknet-devnet is running
    When I use account "devnet-1" with "--network devnet"
    Then the account should work immediately
    And no account import should be required

  @account @generic
  Scenario: Import existing account with private key
    Given I have an account address "<address>" and private key "<key>"
    When I request to import it as "<name>" of type "<type>"
    Then the account should be imported to the accounts file
    And it should be usable for transactions

  # ==========================================================================
  # Declaration (Generic)
  # ==========================================================================

  @declare @generic
  Scenario: Declare any contract by name
    Given I am using the "<profile>" profile
    And a Scarb project exists with contract "<contract_name>"
    When I request to declare contract "<contract_name>"
    Then the contract class should be declared
    And the class_hash should be returned as "0x..."

    Examples:
      | profile | contract_name |
      | devnet  | MyContract    |
      | sepolia | MyContract    |

  @declare @generic @workspace
  Scenario: Declare contract from specific package in workspace
    Given I am using the "devnet" profile
    And a Scarb workspace exists with package "<package>"
    When I request to declare contract "<contract>" from package "<package>"
    Then the class_hash should be returned

  @declare @generic @idempotent
  Scenario: Re-declaring already declared contract succeeds
    Given contract "<contract>" is already declared on "<network>"
    When I request to declare it again
    Then the existing class_hash should be returned
    And no error should occur

  # ==========================================================================
  # Deployment (Generic)
  # ==========================================================================

  @deploy @generic
  Scenario: Deploy contract with no constructor args
    Given I have a class_hash "<class_hash>"
    When I request to deploy it with no constructor calldata
    Then the contract should be deployed
    And the contract address should be returned

  @deploy @generic
  Scenario: Deploy contract with constructor calldata
    Given I have a class_hash for a contract with constructor
    When I request to deploy with calldata "<calldata>"
    Then the contract should be deployed
    And the constructor should receive the arguments

    Examples:
      | calldata           | description              |
      | 0x123              | single felt252           |
      | 0x1 0x2 0x0        | felt + u256              |
      | 3 0xa 0xb 0xc      | array of 3 elements      |

  @deploy @generic @deterministic
  Scenario: Deploy with salt for predictable address
    Given I have a class_hash "<class_hash>"
    When I deploy with salt "<salt>" and unique flag
    Then the address should be deterministic
    And deploying again with same salt should fail (already deployed)

  @deploy @generic @by-name
  Scenario: Deploy by contract name (auto-declare)
    Given contract "<contract>" has not been declared
    When I request to deploy by contract name "<contract>"
    Then it should declare first, then deploy
    And both class_hash and address should be returned

  # ==========================================================================
  # Pipeline/Orchestration (Generic)
  # ==========================================================================

  @pipeline @generic
  Scenario: Deploy multiple contracts with dependencies
    Given a deployment manifest with contracts:
      | name       | depends_on | constructor_uses        |
      | ContractA  |            |                         |
      | ContractB  | ContractA  | ContractA.address       |
    When I request pipeline deployment
    Then ContractA should deploy first
    And ContractB should receive ContractA's address
    And a deployments.json should be created

  @pipeline @generic @idempotent
  Scenario: Pipeline skips already-deployed contracts
    Given ContractA is already deployed (in deployments.json)
    When I run the pipeline again
    Then ContractA should be skipped
    And only new contracts should deploy

  # ==========================================================================
  # Configuration & Error Handling (Generic)
  # ==========================================================================

  @config @generic
  Scenario: Environment variables are resolved in snfoundry.toml
    Given snfoundry.toml contains "$MY_RPC_URL"
    And MY_RPC_URL is set in the environment
    When sncast runs
    Then it should use the resolved URL

  @error @generic
  Scenario: Clear error on network failure
    Given the RPC endpoint is unreachable
    When I attempt any sncast operation
    Then a clear network error should be displayed
    And the failed URL should be shown

  @error @generic
  Scenario: Clear error on compilation failure
    Given the contract has Cairo syntax errors
    When I attempt to declare
    Then scarb build should fail first
    And the compilation error should be displayed
