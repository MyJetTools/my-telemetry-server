use my_telemetry_ui_shared::ProductScopesHttpModel;

use crate::app::AppContext;

use super::FlowError;

/// The product scopes the env has metrics of, the default one first.
pub async fn get_product_scopes(
    app: &AppContext,
    env: &str,
) -> Result<ProductScopesHttpModel, FlowError> {
    let grpc_client = crate::scripts::get_grpc_client(app, env).await?;

    let scopes = grpc_client.get_product_scopes(()).await?;

    Ok(scopes.into())
}
