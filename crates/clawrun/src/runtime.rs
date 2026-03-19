use tracing::warn;

use crate::{
    config::ClawRunConfig,
    copilot_sdk::CopilotSdkAgent,
    types::{AgentEngine, AgentSpec, InferenceRequest, InferenceResult},
};

#[derive(Debug, Clone)]
pub struct ClawRun {
    copilot_agent: CopilotSdkAgent,
}

impl ClawRun {
    pub fn new(cfg: ClawRunConfig) -> Result<Self, String> {
        let copilot_agent = CopilotSdkAgent::new(cfg.copilot)?;
        Ok(Self { copilot_agent })
    }

    pub async fn generate_reply(
        &self,
        agents: &[AgentSpec],
        req: &InferenceRequest,
    ) -> Result<InferenceResult, String> {
        if agents.is_empty() {
            return Err("no agents configured".to_string());
        }

        let agent = select_agent(agents, &req.channel_id, &req.text);

        let output_text = match agent.engine {
            AgentEngine::Template => apply_template(&agent.reply_template, &agent.name, &req.text),
            AgentEngine::CopilotSdk => {
                match self.copilot_agent.run(&agent.name, req).await {
                    Ok(text) => text,
                    Err(err) => {
                        warn!(error = %err, agent = %agent.name, "copilot sdk agent failed, falling back to template");
                        apply_template(&agent.reply_template, &agent.name, &req.text)
                    }
                }
            }
        };

        Ok(InferenceResult {
            agent_name: agent.name.clone(),
            output_text,
        })
    }
}

pub fn select_agent<'a>(agents: &'a [AgentSpec], channel_id: &str, text: &str) -> &'a AgentSpec {
    let lowered = text.to_lowercase();

    agents
        .iter()
        .max_by_key(|agent| {
            let mut score = 0_i64;

            if agent.channels.is_empty() || agent.channels.iter().any(|c| c == channel_id) {
                score += 10;
            }

            let keyword_hits = agent
                .keywords
                .iter()
                .filter(|k| lowered.contains(&k.to_lowercase()))
                .count() as i64;

            score + keyword_hits * 5
        })
        .unwrap_or_else(|| &agents[0])
}

fn apply_template(template: &str, agent: &str, text: &str) -> String {
    template
        .replace("{agent}", agent)
        .replace("{text}", text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_by_keyword() {
        let agents = vec![
            AgentSpec {
                name: "default".to_string(),
                channels: vec![],
                keywords: vec![],
                reply_template: "{text}".to_string(),
                engine: AgentEngine::Template,
            },
            AgentSpec {
                name: "billing".to_string(),
                channels: vec!["qq".to_string()],
                keywords: vec!["refund".to_string()],
                reply_template: "{text}".to_string(),
                engine: AgentEngine::Template,
            },
        ];

        let picked = select_agent(&agents, "qq", "need refund");
        assert_eq!(picked.name, "billing");
    }
}
