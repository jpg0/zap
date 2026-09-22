//! The "Agent providers" settings page: user-configured model providers that the client calls
//! directly (BYOP), without routing inference through Warp's servers.

use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use super::agent_providers_widget::AgentProvidersWidget;
use super::settings_page::{
    MatchData, PageTitle, PageType, SettingsPageMeta, SettingsPageViewHandle, SettingsWidget,
};
use super::SettingsSection;
use crate::settings::AISettings;

const PAGE_TITLE: &str = "Agent providers";

pub struct AgentProvidersPageView {
    page: PageType<Self>,
}

impl AgentProvidersPageView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self {
            page: Self::build_page(ctx),
        }
    }

    fn build_page(ctx: &mut ViewContext<Self>) -> PageType<Self> {
        let widgets: Vec<Box<dyn SettingsWidget<View = Self>>> =
            vec![Box::new(AgentProvidersWidget::new(ctx))];
        PageType::new_uncategorized(widgets, Some(PageTitle::new(PAGE_TITLE)))
    }

    pub fn rebuild_current_page(&mut self, ctx: &mut ViewContext<Self>) {
        let preserved_scroll = self.page.scroll_states();
        self.page = Self::build_page(ctx);
        if let Some((v, h)) = preserved_scroll {
            self.page.replace_scroll_states(v, h);
        }
        ctx.notify();
    }

    fn save_agent_provider_edits(
        provider_id: &str,
        name: &str,
        base_url: &str,
        api_key: &str,
        headers: &[(String, String)],
        models: &[(usize, String, String, u32, u32)],
        ctx: &mut ViewContext<Self>,
    ) {
        AISettings::handle(ctx).update(ctx, |settings, ctx| {
            let mut providers = settings.agent_providers.value().clone();
            if let Some(p) = providers.iter_mut().find(|p| p.id == provider_id) {
                p.name = name.to_owned();
                p.base_url = base_url.to_owned();
                p.extra_headers = headers.to_vec();
                // 按 model_index 更新，跳过越界索引（rebuild 中间表单与 settings 可能短暂不一致）。
                for (idx, m_name, m_id, ctx_window, max_out) in models {
                    if let Some(m) = p.models.get_mut(*idx) {
                        m.name = m_name.clone();
                        m.id = m_id.clone();
                        m.context_window = *ctx_window;
                        m.max_output_tokens = *max_out;
                    }
                }
            }
            let _ = settings.agent_providers.set_value(providers, ctx);
        });
        crate::ai::agent_providers::AgentProviderSecrets::handle(ctx).update(
            ctx,
            |secrets, ctx| {
                secrets.set(provider_id, api_key.to_owned(), ctx);
            },
        );
    }
}

impl Entity for AgentProvidersPageView {
    type Event = ();
}

impl View for AgentProvidersPageView {
    fn ui_name() -> &'static str {
        "AgentProvidersPage"
    }

    fn render(&self, app: &AppContext) -> Box<dyn warpui::elements::Element> {
        self.page.render(self, app)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentProvidersPageAction {
    AddAgentProvider,
    RemoveAgentProvider {
        provider_id: String,
    },
    UpdateAgentProviderName {
        provider_id: String,
        name: String,
    },
    UpdateAgentProviderBaseUrl {
        provider_id: String,
        base_url: String,
    },
    /// 显式设置 provider 的 API 协议类型(OpenAI / OpenAI-Response / Gemini / Anthropic / Ollama)。
    /// chat_stream 据此显式绑定 genai AdapterKind,绕过模型名识别。
    SetAgentProviderApiType {
        provider_id: String,
        api_type: crate::settings::AgentProviderApiType,
    },
    UpdateAgentProviderApiKey {
        provider_id: String,
        api_key: String,
    },
    /// 一次性保存某个 provider 卡片上的全部可编辑字段(name / base_url / api_key /
    /// extra_headers / models)。取代原来"失焦/Enter 逐字段推入"的 UX —— 用户在
    /// settings_view 点"保存"按钮后一起下发。
    SaveAgentProviderEdits {
        provider_id: String,
        name: String,
        base_url: String,
        api_key: String,
        headers: Vec<(String, String)>,
        /// 只携带可编辑部分:`(model_index, name, id, context_window, max_output_tokens)`。
        /// reasoning / tool_call / image / pdf / audio 由独立的 chip 动作维护,不走这里。
        models: Vec<(usize, String, String, u32, u32)>,
    },
    SaveAgentProviderEditsThen {
        provider_id: String,
        name: String,
        base_url: String,
        api_key: String,
        headers: Vec<(String, String)>,
        /// 只携带可编辑部分:`(model_index, name, id, context_window, max_output_tokens)`。
        models: Vec<(usize, String, String, u32, u32)>,
        action: Box<AgentProvidersPageAction>,
    },
    UpdateAgentProviderModels {
        provider_id: String,
        models: Vec<crate::settings::AgentProviderModel>,
    },
    AddAgentProviderModel {
        provider_id: String,
    },
    RemoveAgentProviderModel {
        provider_id: String,
        model_index: usize,
    },
    UpdateAgentProviderModelName {
        provider_id: String,
        model_index: usize,
        name: String,
    },
    UpdateAgentProviderModelId {
        provider_id: String,
        model_index: usize,
        id: String,
    },
    /// 更新单条模型的 context_window(tokens),0 = 未指定。
    UpdateAgentProviderModelContextWindow {
        provider_id: String,
        model_index: usize,
        context_window: u32,
    },
    /// 更新单条模型的 max_output_tokens,0 = 未指定。
    UpdateAgentProviderModelMaxOutput {
        provider_id: String,
        model_index: usize,
        max_output_tokens: u32,
    },
    AddAgentProviderHeader {
        provider_id: String,
    },
    RemoveAgentProviderHeader {
        provider_id: String,
        header_index: usize,
    },
    UpdateAgentProviderHeader {
        provider_id: String,
        header_index: usize,
        key: String,
        value: String,
    },
    FetchAgentProviderModels {
        provider_id: String,
    },
    /// 触发一次 models.dev 目录加载(磁盘缓存 + 必要时网络刷新)。Providers 子页打开即触发。
    EnsureModelsDevLoaded,
    /// 强制刷新 models.dev 目录(忽略 TTL)。"刷新" 按钮触发。
    RefreshModelsDev,
    /// 从 models.dev 目录创建一个新 provider:回填 name/base_url/全部模型(含 context)。
    AddProviderFromModelsDev {
        catalog_provider_id: String,
    },
    /// 把现有 provider 的模型列表与 models.dev 同步(按 base_url 匹配),
    /// 用 catalog 提供的 context_window / reasoning / tool_call 等元数据填充本地条目。
    SyncProviderModelsFromModelsDev {
        provider_id: String,
    },
    /// 折叠/展开 "快速添加" chip 行。
    ToggleModelsDevChipsExpanded,
    /// 设置 "快速添加" chip 行的搜索 query(子串过滤 provider name/id)。
    SetModelsDevSearchQuery(String),

    // ----- 单条模型条目 detail panel -----
    /// 切换单条模型的 detail panel 展开/折叠状态。
    ToggleAgentProviderModelExpanded {
        provider_id: String,
        model_index: usize,
    },
    /// 三态循环切换单条模型的某个多模态 capability(image/pdf/audio)。
    /// `None → Some(true) → Some(false) → None`。
    CycleAgentProviderModelCapability {
        provider_id: String,
        model_index: usize,
        kind: ModelCapabilityKind,
    },
    /// 切换单条模型的 reasoning 标志(普通 bool 字段,不是三态)。
    ToggleAgentProviderModelReasoning {
        provider_id: String,
        model_index: usize,
    },
    /// 切换单条模型的 tool_call 标志。
    ToggleAgentProviderModelToolCall {
        provider_id: String,
        model_index: usize,
    },
}

/// model detail panel 三态 capability chip 的种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCapabilityKind {
    Image,
    Pdf,
    Audio,
}

impl TypedActionView for AgentProvidersPageView {
    type Action = AgentProvidersPageAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AgentProvidersPageAction::AddAgentProvider => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    providers.push(crate::settings::AgentProvider::new_empty());
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::RemoveAgentProvider { provider_id } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    providers.retain(|p| p.id != *provider_id);
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                crate::ai::agent_providers::AgentProviderSecrets::handle(ctx).update(
                    ctx,
                    |secrets, ctx| {
                        secrets.remove(provider_id, ctx);
                    },
                );
                super::agent_providers_widget::clear_expanded_models_for_provider(provider_id);
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::UpdateAgentProviderName { provider_id, name } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.name = name.clone();
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::UpdateAgentProviderBaseUrl {
                provider_id,
                base_url,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.base_url = base_url.clone();
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::SetAgentProviderApiType {
                provider_id,
                api_type,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.api_type = *api_type;
                        // 若 base_url 为空,顺手填该类型的默认 endpoint(便于新手)。
                        // 用户已自填 base_url 时不动。
                        if p.base_url.trim().is_empty() {
                            p.base_url = api_type.default_base_url().to_owned();
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::UpdateAgentProviderApiKey {
                provider_id,
                api_key,
            } => {
                crate::ai::agent_providers::AgentProviderSecrets::handle(ctx).update(
                    ctx,
                    |secrets, ctx| {
                        secrets.set(provider_id, api_key.clone(), ctx);
                    },
                );
                ctx.notify();
            }
            AgentProvidersPageAction::SaveAgentProviderEdits {
                provider_id,
                name,
                base_url,
                api_key,
                headers,
                models,
            } => {
                Self::save_agent_provider_edits(
                    provider_id,
                    name,
                    base_url,
                    api_key,
                    headers,
                    models,
                    ctx,
                );
                ctx.notify();
            }
            AgentProvidersPageAction::SaveAgentProviderEditsThen {
                provider_id,
                name,
                base_url,
                api_key,
                headers,
                models,
                action,
            } => {
                Self::save_agent_provider_edits(
                    provider_id,
                    name,
                    base_url,
                    api_key,
                    headers,
                    models,
                    ctx,
                );
                self.handle_action(action.as_ref(), ctx);
            }
            AgentProvidersPageAction::UpdateAgentProviderModels {
                provider_id,
                models,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.models = models.clone();
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::AddAgentProviderModel { provider_id } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.models
                            .push(crate::settings::AgentProviderModel::from_id(String::new()));
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                // 行级 add 需要新建 EditorView,所以走 rebuild;rebuild_current_page 已保留滚动。
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::RemoveAgentProviderModel {
                provider_id,
                model_index,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if *model_index < p.models.len() {
                            p.models.remove(*model_index);
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                // 删一条会让后续 index 漂移,清掉这个 provider 的全部展开记录避免误展开。
                super::agent_providers_widget::clear_expanded_models_for_provider(provider_id);
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::UpdateAgentProviderModelName {
                provider_id,
                model_index,
                name,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.name = name.clone();
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::UpdateAgentProviderModelId {
                provider_id,
                model_index,
                id,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.id = id.clone();
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::UpdateAgentProviderModelContextWindow {
                provider_id,
                model_index,
                context_window,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.context_window = *context_window;
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::UpdateAgentProviderModelMaxOutput {
                provider_id,
                model_index,
                max_output_tokens,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.max_output_tokens = *max_output_tokens;
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::FetchAgentProviderModels { provider_id } => {
                let provider_id = provider_id.clone();
                let providers = AISettings::as_ref(ctx).agent_providers.value().clone();
                let Some(provider) = providers.into_iter().find(|p| p.id == provider_id) else {
                    return;
                };
                let api_key = crate::ai::agent_providers::AgentProviderSecrets::as_ref(ctx)
                    .get(&provider_id)
                    .map(str::to_owned);
                let client = http_client::Client::new();
                let provider_id_for_handler = provider_id.clone();
                ctx.spawn(
                    async move {
                        crate::ai::agent_providers::fetch_openai_compatible_models(
                            client,
                            &provider.base_url,
                            api_key.as_deref(),
                        )
                        .await
                    },
                    move |view, result, ctx| match result {
                        Ok(fetched) => {
                            AISettings::handle(ctx).update(ctx, |settings, ctx| {
                                let mut providers = settings.agent_providers.value().clone();
                                if let Some(p) = providers
                                    .iter_mut()
                                    .find(|p| p.id == provider_id_for_handler)
                                {
                                    // 合并保留: 已存在的 id 保留用户改过的 name,新 id 追加,
                                    // 本地多余的 id 不删(用户手动 ×)。
                                    let existing: std::collections::HashSet<String> =
                                        p.models.iter().map(|m| m.id.clone()).collect();
                                    for m in fetched {
                                        if !existing.contains(&m.id) {
                                            p.models.push(
                                                crate::settings::AgentProviderModel::from_id(m.id),
                                            );
                                        }
                                    }
                                }
                                let _ = settings.agent_providers.set_value(providers, ctx);
                            });
                            // 模型行数可能变了,需要 rebuild widget rows。
                            view.rebuild_current_page(ctx);
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to fetch models for provider {provider_id_for_handler}: {e}"
                            );
                            ctx.notify();
                        }
                    },
                );
            }
            AgentProvidersPageAction::AddAgentProviderHeader { provider_id } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        p.extra_headers.push((String::new(), String::new()));
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                // header 行数量变化后需要新建/销毁 EditorView handle,仅 notify 不会刷新 rows。
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::RemoveAgentProviderHeader {
                provider_id,
                header_index,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if *header_index < p.extra_headers.len() {
                            p.extra_headers.remove(*header_index);
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                // 删除同样会导致 index 与现有 HeaderRow handle 漂移,需要重建页面。
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::UpdateAgentProviderHeader {
                provider_id,
                header_index,
                key,
                value,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(h) = p.extra_headers.get_mut(*header_index) {
                            *h = (key.clone(), value.clone());
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                ctx.notify();
            }
            AgentProvidersPageAction::EnsureModelsDevLoaded => {
                use crate::ai::agent_providers::models_dev;
                let had_disk = models_dev::load_from_disk();
                if !had_disk || models_dev::is_stale() {
                    let client = http_client::Client::new();
                    ctx.spawn(
                        async move { models_dev::fetch_and_cache(client).await },
                        |view, result, ctx| match result {
                            Ok(()) => {
                                models_dev::set_fetch_failed(false);
                                view.rebuild_current_page(ctx);
                            }
                            Err(e) => {
                                log::warn!("[models.dev] 拉取失败: {e}");
                                models_dev::set_fetch_failed(true);
                                ctx.notify();
                            }
                        },
                    );
                } else {
                    ctx.notify();
                }
            }
            AgentProvidersPageAction::RefreshModelsDev => {
                use crate::ai::agent_providers::models_dev;
                let client = http_client::Client::new();
                ctx.spawn(
                    async move { models_dev::fetch_and_cache(client).await },
                    |view, result, ctx| match result {
                        Ok(()) => {
                            models_dev::set_fetch_failed(false);
                            view.rebuild_current_page(ctx);
                        }
                        Err(e) => {
                            log::warn!("[models.dev] 刷新失败: {e}");
                            models_dev::set_fetch_failed(true);
                            ctx.notify();
                        }
                    },
                );
            }
            AgentProvidersPageAction::AddProviderFromModelsDev {
                catalog_provider_id,
            } => {
                use crate::ai::agent_providers::models_dev;
                let Some(catalog) = models_dev::cached() else {
                    log::warn!("[models.dev] 目录尚未加载,无法添加 {catalog_provider_id}");
                    return;
                };
                let Some(cat_provider) = catalog.get(catalog_provider_id) else {
                    log::warn!("[models.dev] 目录中无 provider id: {catalog_provider_id}");
                    return;
                };
                let mut new_provider = crate::settings::AgentProvider::new_empty();
                new_provider.name = if cat_provider.name.is_empty() {
                    catalog_provider_id.clone()
                } else {
                    cat_provider.name.clone()
                };
                if let Some(api) = &cat_provider.api {
                    new_provider.base_url = api.clone();
                }
                new_provider.models = cat_provider
                    .models
                    .values()
                    .map(models_dev::into_agent_provider_model)
                    .collect();
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    providers.push(new_provider);
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::SyncProviderModelsFromModelsDev { provider_id } => {
                use crate::ai::agent_providers::models_dev;
                let Some(catalog) = models_dev::cached() else {
                    log::warn!("[models.dev] 目录未加载,无法同步 {provider_id}");
                    return;
                };
                let providers_snapshot = AISettings::as_ref(ctx).agent_providers.value().clone();
                let Some(local) = providers_snapshot.iter().find(|p| p.id == *provider_id) else {
                    return;
                };
                // 匹配策略:先按 base_url 完全相等 / 包含;否则按 name (大小写无关) 匹配 catalog provider id 或 name。
                let target_url = local.base_url.trim().trim_end_matches('/').to_lowercase();
                let target_name = local.name.trim().to_lowercase();
                let cat_provider = catalog.iter().find(|(_, p)| {
                    if let Some(api) = &p.api {
                        let api_norm = api.trim().trim_end_matches('/').to_lowercase();
                        if !target_url.is_empty()
                            && (api_norm == target_url
                                || api_norm.contains(&target_url)
                                || target_url.contains(&api_norm))
                        {
                            return true;
                        }
                    }
                    !target_name.is_empty()
                        && (p.name.to_lowercase() == target_name
                            || p.id.to_lowercase() == target_name)
                });
                let Some((_, cat_provider)) = cat_provider else {
                    log::warn!(
                        "[models.dev] 未在目录中找到匹配 (base_url={}, name={})",
                        local.base_url,
                        local.name
                    );
                    return;
                };
                let cat_models = cat_provider.models.clone();
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        // 既有 id 用 catalog 元数据覆盖;catalog 多出的追加;本地多出的(用户自定义)保留。
                        for local_model in p.models.iter_mut() {
                            if let Some(cat_m) = cat_models.get(&local_model.id) {
                                let merged = models_dev::into_agent_provider_model(cat_m);
                                local_model.context_window = merged.context_window;
                                local_model.max_output_tokens = merged.max_output_tokens;
                                local_model.reasoning = merged.reasoning;
                                local_model.tool_call = merged.tool_call;
                                if local_model.name.trim().is_empty() {
                                    local_model.name = merged.name;
                                }
                                // 多模态 capability:**只填 None 槽位**,Some(_) 视为用户
                                // 已显式覆盖,sync 不动。这样:
                                // - 首次 sync(用户没碰过) → 全部写入 catalog 推断结果
                                // - 用户手动 cycle 到 Some(true/false) 后再 sync → 保留覆盖
                                // - 用户三态循环回 None(Auto) → 下次 sync 又会被填上
                                if local_model.image.is_none() {
                                    local_model.image = merged.image;
                                }
                                if local_model.pdf.is_none() {
                                    local_model.pdf = merged.pdf;
                                }
                                if local_model.audio.is_none() {
                                    local_model.audio = merged.audio;
                                }
                            }
                        }
                        let existing: std::collections::HashSet<String> =
                            p.models.iter().map(|m| m.id.clone()).collect();
                        for cat_m in cat_models.values() {
                            if !existing.contains(&cat_m.id) {
                                p.models.push(models_dev::into_agent_provider_model(cat_m));
                            }
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::ToggleModelsDevChipsExpanded => {
                use crate::ai::agent_providers::models_dev;
                models_dev::toggle_chips_expanded();
                ctx.notify();
            }
            AgentProvidersPageAction::SetModelsDevSearchQuery(q) => {
                use crate::ai::agent_providers::models_dev;
                models_dev::set_search_query(q.clone());
                ctx.notify();
            }
            AgentProvidersPageAction::ToggleAgentProviderModelExpanded {
                provider_id,
                model_index,
            } => {
                super::agent_providers_widget::toggle_model_expanded(provider_id, *model_index);
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::CycleAgentProviderModelCapability {
                provider_id,
                model_index,
                kind,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            let slot = match kind {
                                ModelCapabilityKind::Image => &mut m.image,
                                ModelCapabilityKind::Pdf => &mut m.pdf,
                                ModelCapabilityKind::Audio => &mut m.audio,
                            };
                            // 三态循环:None → Some(true) → Some(false) → None。
                            *slot = match *slot {
                                None => Some(true),
                                Some(true) => Some(false),
                                Some(false) => None,
                            };
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::ToggleAgentProviderModelReasoning {
                provider_id,
                model_index,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.reasoning = !m.reasoning;
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
            AgentProvidersPageAction::ToggleAgentProviderModelToolCall {
                provider_id,
                model_index,
            } => {
                AISettings::handle(ctx).update(ctx, |settings, ctx| {
                    let mut providers = settings.agent_providers.value().clone();
                    if let Some(p) = providers.iter_mut().find(|p| p.id == *provider_id) {
                        if let Some(m) = p.models.get_mut(*model_index) {
                            m.tool_call = !m.tool_call;
                        }
                    }
                    let _ = settings.agent_providers.set_value(providers, ctx);
                });
                self.rebuild_current_page(ctx);
            }
        }
    }
}

impl SettingsPageMeta for AgentProvidersPageView {
    fn section() -> SettingsSection {
        SettingsSection::AgentProviders
    }

    fn should_render(&self, _ctx: &AppContext) -> bool {
        FeatureFlag::AgentMode.is_enabled()
    }

    fn update_filter(&mut self, query: &str, ctx: &mut ViewContext<Self>) -> MatchData {
        self.page.update_filter(query, ctx)
    }

    fn scroll_to_widget(&mut self, widget_id: &'static str) {
        self.page.scroll_to_widget(widget_id)
    }

    fn clear_highlighted_widget(&mut self) {
        self.page.clear_highlighted_widget();
    }
}

impl From<ViewHandle<AgentProvidersPageView>> for SettingsPageViewHandle {
    fn from(view_handle: ViewHandle<AgentProvidersPageView>) -> Self {
        SettingsPageViewHandle::AgentProviders(view_handle)
    }
}
