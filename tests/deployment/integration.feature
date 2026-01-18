Feature: Katana-TEE Contract Deployment Integration
  Test the deployment skills using amd_tee_registry and katana_tee

  Background:
    Given starknet-devnet is running with seed 42
    And I am using account "devnet-1"

  @integration @local
  Scenario: Deploy amd_tee_registry with empty arrays
    Given the amd_tee_registry contract is compiled
    When I declare "AMDTEERegistry" from package "amd_tee_registry"
    And I deploy with calldata "0 0 0 0 0 0 0"
    Then the contract should be deployed
    And I should be able to call "is_trusted_intermediate_cert"

  @integration @local
  Scenario: Deploy katana_tee with registry dependency
    Given amd_tee_registry is deployed at "<registry_address>"
    When I declare "KatanaTee" from package "katana_tee"
    And I deploy with calldata "<registry_address>"
    Then katana_tee should be deployed
    And calling "get_registry_address" should return "<registry_address>"

  @integration @local @full-pipeline
  Scenario: Full deployment of both contracts
    When I run the deployment pipeline for this project
    Then both contracts should be deployed in order
    And deployments/devnet.json should contain both addresses
    And katana_tee.registry_address == amd_tee_registry.address
