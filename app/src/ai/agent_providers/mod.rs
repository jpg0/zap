//! 自定义 Agent 提供商支持。
//!
//! 这个模块负责:
//! - 把每个 Provider 的 `api_key` 安全地存到 OS keychain (secure_storage),
//!   而 Provider 元数据(name/base_url/model 列表) 走普通 settings.toml。
//! - 通过 `OpenAiCompatibleClient` 调用 `${base_url}/models`
//!   抓取上游可用模型列表(供 UI "Fetch models" 按钮使用)。
//!
//! 第二阶段会基于这套配置实现 `AiProvider` trait,
//! 把 Agent 的 multi-agent 调用分流到本地 Provider。

pub mod active_ai;
pub mod attachment_caps;
pub mod chat_stream;
pub mod content_tool_calls;
pub mod llm_id;
pub mod models_dev;
pub mod oneshot;
pub mod openai_compatible;
pub mod prompt_renderer;
pub mod reasoning;
pub mod secrets;
pub mod tools;
pub mod user_context;

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;

#[cfg(test)]
mod cache_stability_tests;

// 当前外部使用点:
// - `fetch_openai_compatible_models`: ai_page.rs 中的 FetchAgentProviderModels handler
// - `AgentProviderSecrets`: ai_page.rs 中的多个 handler 与 lib.rs 注册点
// 其余符号(`OpenAiCompatibleError`/`OpenAiCompatibleModel`/`AgentProviderSecretsEvent`)
// 仍可通过 `crate::ai::agent_providers::openai_compatible::*` 等完整路径访问,
// 这里不再 re-export 以避免 `unused_imports` 告警。
pub use openai_compatible::fetch_openai_compatible_models;
pub use secrets::AgentProviderSecrets;

// ---------------------------------------------------------------------------
// LLMInfo 合成:把 settings 中配置的 agent_providers 转成 picker 可用的形态
// ---------------------------------------------------------------------------

use std::collections::HashMap;

use settings::Setting;
use warpui::{AppContext, SingletonEntity};

use crate::ai::llms::{LLMContextWindow, LLMInfo, LLMProvider, LLMUsageMetadata};
use crate::settings::{AISettings, AgentProvider};

/// 合成给定 provider 的所有合法 (provider, model) 对的 LLMInfo 列表。
///
/// "合法" = provider 有非空 base_url + 至少 1 个 model。
/// **API key 可选**:本地无认证 provider(ollama / lm-studio / vllm 等)允许留空,
/// 缺 key 时仍然把模型暴露给 picker;运行时仍会发请求,只是不带 `Authorization`。
/// 不合法的 provider(没填 base_url 或没模型)会整体被忽略,picker 中不展示其下的模型,
/// 这样用户能直观地看到"哪些 provider 没填全 → 没出现"。
pub fn build_byop_llm_infos(app: &AppContext) -> Vec<LLMInfo> {
    let providers = AISettings::as_ref(app).agent_providers.value().clone();
    let mut out = Vec::new();

    for provider in providers {
        if provider.base_url.trim().is_empty() {
            continue;
        }
        if provider.models.is_empty() {
            continue;
        }

        let provider_label = if provider.name.trim().is_empty() {
            provider.id.clone()
        } else {
            provider.name.clone()
        };

        for model in &provider.models {
            if model.id.trim().is_empty() {
                continue;
            }
            let display_name = if model.name.trim().is_empty() {
                model.id.clone()
            } else {
                model.name.clone()
            };
            // 三层优先级解析最终能力:用户在 settings 三态 chip 强制开关 →
            // models.dev catalog 推断 → substring fallback。
            // 这个函数也是 chat_stream 决策塞 ContentPart::Binary 时用的同一个,
            // UI 显示与运行时行为永远一致。
            let resolved_caps =
                attachment_caps::resolve_for_model(&provider.id, provider.api_type, model);
            let vision_supported = resolved_caps.images;
            out.push(LLMInfo {
                display_name: format!("{provider_label} / {display_name}"),
                base_model_name: format!("{provider_label} / {display_name}"),
                id: llm_id::encode(&provider.id, &model.id),
                reasoning_level: None,
                usage_metadata: LLMUsageMetadata {
                    request_multiplier: 1,
                    credit_multiplier: None,
                },
                description: None,
                disable_reason: None,
                vision_supported,
                spec: None,
                provider: LLMProvider::Unknown,
                host_configs: HashMap::new(),
                discount_percentage: None,
                context_window: LLMContextWindow::default(),
            });
        }
    }

    out
}

/// Whether the user has configured any provider of their own. Requests to those providers are
/// sent from this client with the user's own credentials and never reach Warp, so credit, plan,
/// and workspace-policy gates do not apply to them.
pub fn has_configured_providers(app: &AppContext) -> bool {
    !AISettings::as_ref(app).agent_providers.value().is_empty()
}

/// 给定一个 BYOP `LLMId`,从 `AISettings` 与 secrets 里查出 `(provider, api_key, model_id)`。
/// 任一信息缺失返回 `None`(controller 调用方应映射为 `InvalidApiKey` 错误)。
pub fn lookup_byop(app: &AppContext, id: &ai::LLMId) -> Option<(AgentProvider, String, String)> {
    let (provider_id, model_id) = llm_id::decode(id)?;
    let providers = AISettings::as_ref(app).agent_providers.value().clone();
    let provider = providers.into_iter().find(|p| p.id == provider_id)?;
    // API key 可选:无 key 时返回空字符串,下游 build_client 会传给 genai
    // `AuthData::from_single("")` —— 不附带 `Authorization`,适配 ollama 等本地无认证服务。
    let api_key = AgentProviderSecrets::as_ref(app)
        .get(&provider_id)
        .map(str::to_owned)
        .unwrap_or_default();
    Some((provider, api_key, model_id))
}
