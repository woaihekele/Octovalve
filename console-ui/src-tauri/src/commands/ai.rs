use crate::services::ai_risk;
use crate::types::AiRiskRequest;

#[tauri::command]
pub async fn ai_risk_assess(
    request: AiRiskRequest,
) -> Result<crate::types::AiRiskResponse, String> {
    ai_risk::ai_risk_assess(request).await
}
