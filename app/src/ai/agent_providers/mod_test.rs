//! Smoke tests for BYOP provider configuration and lookup.

use ai::LLMId;
use settings::Setting;
use warpui::{App, SingletonEntity};

use crate::ai::agent_providers::{llm_id, lookup_byop, AgentProviderSecrets};
use crate::ai::llms::LLMPreferences;
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::ai::mcp::TemplatableMCPServerManager;
use crate::auth::AuthStateProvider;
use crate::auth::auth_manager::AuthManager;
use crate::cloud_object::model::persistence::CloudModel;
use crate::network::NetworkStatus;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::server_api::ServerApiProvider;
use crate::server::sync_queue::SyncQueue;
use crate::workspaces::team_tester::TeamTesterStatus;
use crate::LaunchMode;
use crate::settings::AISettings;
use crate::test_util::byop::{init_byop_test_app, sample_provider};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::workspaces::user_workspaces::{TeamlessScopeForTest, UserWorkspaces};


fn byop_choice_ids(ctx: &warpui::AppContext) -> Vec<LLMId> {
    LLMPreferences::as_ref(ctx)
        .byop_llm_choices()
        .map(|info| info.id.clone())
        .collect()
}

fn agent_mode_choice_ids(ctx: &warpui::AppContext) -> Vec<LLMId> {
    LLMPreferences::as_ref(ctx)
        .get_base_llm_choices_for_agent_mode(&TeamlessScopeForTest, ctx)
        .map(|info| info.id.clone())
        .collect()
}

fn cli_agent_choice_ids(ctx: &warpui::AppContext) -> Vec<LLMId> {
    LLMPreferences::as_ref(ctx)
        .get_cli_agent_llm_choices(&TeamlessScopeForTest, ctx)
        .map(|info| info.id.clone())
        .collect()
}

#[test]
fn smoke_configured_byop_models_appear_in_picker() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        let provider_id = "provider-smoke-1";
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider(provider_id)], ctx);
            });
        });

        app.read(|ctx| {
            let expected = llm_id::encode(provider_id, "llama3.2");
            assert_eq!(byop_choice_ids(ctx), vec![expected.clone()]);
            assert!(
                agent_mode_choice_ids(ctx).contains(&expected),
                "a configured provider model belongs in the agent mode picker"
            );
            assert!(
                !cli_agent_choice_ids(ctx).contains(&expected),
                "the CLI agent model is resolved by Warp, which cannot use a provider model"
            );
            let info = LLMPreferences::as_ref(ctx)
                .custom_llm_info_for_id(&expected)
                .expect("BYOP model should resolve by id");
            assert!(info.disable_reason.is_none(), "valid provider should not be disabled");
        });
    });
}

#[test]
fn smoke_no_byop_models_without_providers() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);
        app.read(|ctx| assert!(byop_choice_ids(ctx).is_empty()));
    });
}

#[test]
fn smoke_byop_provider_with_empty_base_url_is_skipped() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let mut broken = sample_provider("broken");
                broken.base_url.clear();
                let _ = settings.agent_providers.set_value(vec![broken], ctx);
            });
        });

        app.read(|ctx| {
            assert!(
                byop_choice_ids(ctx).is_empty(),
                "provider with empty base_url must not appear as a selectable model"
            );
        });
    });
}

#[test]
fn smoke_lookup_byop_resolves_provider_and_model_without_api_key() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        let provider_id = "provider-lookup-1";
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider(provider_id)], ctx);
            });
        });

        let encoded = llm_id::encode(provider_id, "llama3.2");
        app.read(|ctx| {
            let (provider, api_key, model_id) =
                lookup_byop(ctx, &encoded).expect("lookup_byop should resolve configured model");
            assert_eq!(provider.id, provider_id);
            assert_eq!(model_id, "llama3.2");
            assert!(api_key.is_empty(), "Ollama path allows empty API key");
        });
    });
}

#[test]
fn smoke_lookup_byop_returns_none_for_unknown_id() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        app.read(|ctx| {
            assert!(lookup_byop(ctx, &LLMId::from("byop:missing:model")).is_none());
            assert!(lookup_byop(ctx, &LLMId::from("not-byop")).is_none());
        });
    });
}

/// Requests to the user's own providers never reach Warp, so configuring one enables the agent
/// even for a logged-out user, who would otherwise have AI disabled entirely.
#[test]
fn own_providers_enable_ai_while_logged_out() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(AgentProviderSecrets::new);
        app.add_singleton_model(|_| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_logged_out_for_test());
        app.add_singleton_model(AuthManager::new_for_test);
        app.add_singleton_model(|_| NetworkStatus::new());
        app.add_singleton_model(UserWorkspaces::default_mock);
        app.add_singleton_model(CloudModel::mock);
        app.add_singleton_model(TeamTesterStatus::mock);
        app.add_singleton_model(SyncQueue::mock);
        app.add_singleton_model(UpdateManager::mock);
        app.add_singleton_model(|_| TemplatableMCPServerManager::default());
        app.add_singleton_model(|ctx| {
            AIExecutionProfilesModel::new(&LaunchMode::new_for_unit_test(), ctx)
        });
        app.add_singleton_model(LLMPreferences::new);

        app.read(|ctx| {
            assert!(
                AuthStateProvider::as_ref(ctx)
                    .get()
                    .is_anonymous_or_logged_out(),
                "test app should start logged out"
            );
            assert!(
                !AISettings::as_ref(ctx).is_any_ai_enabled(ctx),
                "a logged-out user with no providers has no way to run a model"
            );
        });

        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider("provider-logged-out")], ctx);
            });
        });

        app.read(|ctx| {
            assert!(
                AISettings::as_ref(ctx).is_any_ai_enabled(ctx),
                "a configured provider should enable AI without an account"
            );
        });
    });
}

/// BYOP models are called with the user's own credentials, so no workspace BYO policy gates
/// them the way it gates Warp's custom endpoints.
#[test]
fn byop_models_are_not_gated_by_workspace_policy() {
    App::test((), |mut app| async move {
        init_byop_test_app(&mut app);

        let provider_id = "provider-policy";
        app.update(|ctx| {
            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                let _ = settings
                    .agent_providers
                    .set_value(vec![sample_provider(provider_id)], ctx);
            });
        });

        app.read(|ctx| {
            let preferences = LLMPreferences::as_ref(ctx);
            let encoded = llm_id::encode(provider_id, "llama3.2");
            let info = preferences
                .byop_llm_info_for_id(&encoded)
                .expect("configured BYOP model should resolve");
            let scope = TeamlessScopeForTest;
            assert!(crate::ai::llms::is_model_allowed_for_scope(
                preferences,
                info,
                &scope,
                ctx
            ));
        });
    });
}
