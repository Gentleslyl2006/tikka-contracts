use super::*;
use soroban_sdk::testutils::{MockAuth, MockAuthInvoke};
use soroban_sdk::IntoVal;

#[test]
fn every_factory_admin_entrypoint_succeeds_for_admin() {
    let env = Env::default();
    let (client, _admin, _treasury) = setup_factory(&env);

    for entrypoint in ["pause_factory", "unpause_factory"] {
        match entrypoint {
            "pause_factory" => client.pause_factory(),
            "unpause_factory" => client.unpause_factory(),
            _ => unreachable!(),
        }
    }

    assert!(!client.is_factory_paused());
}

#[test]
fn every_factory_admin_entrypoint_rejects_non_admin() {
    let env = Env::default();
    let (client, _admin, _treasury) = setup_factory(&env);
    let stranger = Address::generate(&env);

    for entrypoint in ["pause_factory", "unpause_factory"] {
        env.mock_auths(&[MockAuth {
            address: &stranger,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: entrypoint,
                args: ().into_val(&env),
                sub_invokes: &[],
            },
        }]);

        let result = match entrypoint {
            "pause_factory" => client.try_pause_factory(),
            "unpause_factory" => client.try_unpause_factory(),
            _ => unreachable!(),
        };
        assert!(result.is_err());
    }
}
