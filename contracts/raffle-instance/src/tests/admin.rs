use super::*;
use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::IntoVal;

#[test]
fn every_admin_entrypoint_succeeds_for_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _creator, _buyer, _factory, _token_mint) = setup_active_raffle(&env);

    for entrypoint in ["set_protocol_fee_bp", "set_swap_deadline"] {
        match entrypoint {
            "set_protocol_fee_bp" => client.set_protocol_fee_bp(&1),
            "set_swap_deadline" => client.set_swap_deadline(&1),
            _ => unreachable!(),
        }
    }

    assert_eq!(client.get_raffle().protocol_fee_bp, 1);
    assert_eq!(client.get_raffle().swap_deadline_seconds, 1);
}

#[test]
fn every_admin_entrypoint_rejects_non_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _creator, _buyer, _factory, _token_mint) = setup_active_raffle(&env);
    let stranger = Address::generate(&env);

    for entrypoint in ["set_protocol_fee_bp", "set_swap_deadline"] {
        let auth = match entrypoint {
            "set_protocol_fee_bp" => MockAuth {
                address: &stranger,
                invoke: &MockAuthInvoke {
                    contract: &client.address,
                    fn_name: entrypoint,
                    args: (1u32,).into_val(&env),
                    sub_invokes: &[],
                },
            },
            "set_swap_deadline" => MockAuth {
                address: &stranger,
                invoke: &MockAuthInvoke {
                    contract: &client.address,
                    fn_name: entrypoint,
                    args: (1u64,).into_val(&env),
                    sub_invokes: &[],
                },
            },
            _ => unreachable!(),
        };
        env.mock_auths(&[auth]);

        let result = match entrypoint {
            "set_protocol_fee_bp" => client.try_set_protocol_fee_bp(&1).map(|_| ()),
            "set_swap_deadline" => client.try_set_swap_deadline(&1).map(|_| ()),
            _ => unreachable!(),
        };
        assert!(result.is_err());
    }
}
