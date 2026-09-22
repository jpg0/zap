use std::sync::Arc;

use ai::LLMProvider;
use warpui::App;

use super::*;
use crate::ai::credit_availability::AICreditSource;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::server::telemetry::context_provider::AppTelemetryContextProvider;
use crate::workspaces::user_workspaces::TeamlessScopeForTest;
use crate::workspaces::workspace::{ByoApiKeyPolicy, Workspace, WorkspaceUid};
use settings::Setting as _;

fn initialize_app(app: &mut App) {
    initialize_app_with_workspaces(app, vec![]);
}

fn initialize_app_with_workspaces(app: &mut App, workspaces: Vec<Workspace>) {
    app.add_singleton_model(|_| NetworkStatus::new());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());
    app.add_singleton_model(|_| ServerApiProvider::new_for_test());
    app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            Arc::new(MockTeamClient::new()),
            Arc::new(MockWorkspaceClient::new()),
            workspaces,
            ctx,
        )
    });
    crate::test_util::settings::initialize_settings_for_tests(app);
    app.add_singleton_model(|_| crate::pricing::PricingInfoModel::new());
    app.add_singleton_model(|ctx| {
        AIRequestUsageModel::new_for_test(ServerApiProvider::as_ref(ctx).get_ai_client(), ctx)
    });
}

fn apply_server_availability(app: &mut App, availability: AICreditAvailability) {
    AIRequestUsageModel::handle(app).update(app, |model, ctx| {
        model.apply_server_availability(Ok(availability), ctx);
    });
}

fn determine_state(app: &mut App) -> PromptAlertState {
    app.read(|ctx| PromptAlertView::determine_state(&TeamlessScopeForTest, ctx))
}

#[test]
fn test_server_available_maps_to_no_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::available_with_source(Some(AICreditSource::BaseLimit)),
        );
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_server_delinquent_maps_to_delinquency_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::Delinquent),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::DelinquentDueToPaymentIssue
        );
    });
}

#[test]
fn test_server_spend_limit_reasons_map_to_spend_limit_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        for reason in [
            AICreditDenialReason::EnterpriseTeamSpendLimitHit,
            AICreditDenialReason::EnterprisePerUserSpendLimitHit,
            AICreditDenialReason::EnterpriseWorkspaceSpendLimitHit,
        ] {
            apply_server_availability(&mut app, AICreditAvailability::unavailable(reason));
            assert_eq!(
                determine_state(&mut app),
                PromptAlertState::MonthlyOveragesSpendLimitReached,
                "unexpected alert state for {reason:?}",
            );
        }
    });
}

#[test]
fn test_server_out_of_credits_maps_to_request_limit_reached() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // With no workspace overage policy in play, an out-of-credits denial
        // falls through to the generic request limit alert.
        for reason in [
            AICreditDenialReason::OutOfCredits,
            AICreditDenialReason::Unknown,
        ] {
            apply_server_availability(&mut app, AICreditAvailability::unavailable(reason));
            assert_eq!(
                determine_state(&mut app),
                PromptAlertState::RequestLimitReached,
                "unexpected alert state for {reason:?}",
            );
        }
    });
}

#[test]
fn test_legacy_fallback_used_before_first_server_response() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // No server availability applied: the default request limit info has
        // requests remaining, so the legacy derivation reports no alert.
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_server_managed_availability_maps_to_no_alert() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        // `available` with no credit source means a server-managed BYO path
        // is configured — definite availability, no local key required.
        apply_server_availability(&mut app, AICreditAvailability::available_with_source(None));
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

#[test]
fn test_out_of_credits_with_local_key_maps_to_no_alert() {
    App::test((), |mut app| async move {
        let uid = WorkspaceUid::from(crate::server::ids::ServerId::from(1_i64));
        let mut workspace =
            Workspace::from_local_cache(uid, "Test Workspace".to_string(), None, None);
        workspace.billing_metadata.tier.byo_api_key_policy =
            Some(ByoApiKeyPolicy { enabled: true });
        initialize_app_with_workspaces(&mut app, vec![workspace]);

        ApiKeyManager::handle(&app).update(&mut app, |manager, ctx| {
            manager.set_provider_key(LLMProvider::OpenAI, Some("test-key".to_string()), ctx);
        });

        // The server cannot see the locally stored key; the client refines
        // its OUT_OF_CREDITS answer.
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::OutOfCredits),
        );
        assert_eq!(determine_state(&mut app), PromptAlertState::NoAlert);
    });
}

/// Requests to a user-configured provider are sent from this client and never reach Warp, so
/// a Warp credit denial must not block them or show a credits alert.
#[test]
fn test_configured_provider_is_not_blocked_by_credit_denial() {
    use crate::settings::{AgentProvider, AgentProviderApiType, AgentProviderModel};

    App::test((), |mut app| async move {
        initialize_app(&mut app);
        apply_server_availability(
            &mut app,
            AICreditAvailability::unavailable(AICreditDenialReason::OutOfCredits),
        );
        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::RequestLimitReached,
            "without a provider, an out-of-credits denial still alerts"
        );
        assert!(app.read(|ctx| PromptAlertView::does_alert_block_ai_requests(
            &TeamlessScopeForTest,
            ctx
        )));

        app.update(|ctx| {
            crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings.agent_providers.set_value(
                    vec![AgentProvider {
                        id: "local".to_owned(),
                        name: "LM Studio".to_owned(),
                        kind: Default::default(),
                        api_type: AgentProviderApiType::OpenAi,
                        base_url: "http://localhost:1234/v1/".to_owned(),
                        models: vec![AgentProviderModel::from_id("local-model".to_owned())],
                        extra_headers: Vec::new(),
                    }],
                    ctx,
                );
            });
        });

        assert_eq!(
            determine_state(&mut app),
            PromptAlertState::NoAlert,
            "a configured provider does not depend on Warp credits"
        );
        assert!(!app.read(|ctx| PromptAlertView::does_alert_block_ai_requests(
            &TeamlessScopeForTest,
            ctx
        )));
    });
}
