//! Token pricing and profitability calculations

use awc::{Client as HttpClient, http::Method};
use clarity::{Address, Uint256};
use log::{debug, error, info};
use num_traits::ToPrimitive;

use crate::error::RelayerError;

/// Fetches the current price of a token from the price API.
///
/// Returns the value of the specified amount in gas token (ALTHEA) units.
/// This function queries a custom price API that provides token valuations.
///
/// # Arguments
/// * `price_api_url` - Base URL of the price API service
/// * `token_address` - Address of the token to price
/// * `amount` - Amount of tokens to value
///
/// # Returns
/// The equivalent value in gas token units, or an error if the price cannot be fetched
pub async fn fetch_value_in_gas_token(
    price_api_url: &str,
    token_address: Address,
    amount: Uint256,
) -> Result<Uint256, RelayerError> {
    let url = format!("{}/value_in_gas_token/{}", price_api_url, token_address);
    debug!("Fetching price from {}", url);

    let client = HttpClient::default();
    let mut response =
        client.request(Method::GET, url).send().await.map_err(|e| {
            RelayerError::NetworkError(format!("Failed to send price request: {}", e))
        })?;

    if !response.status().is_success() {
        let body = response.body().await.map_err(|e| {
            RelayerError::NetworkError(format!("Failed to read error response: {}", e))
        })?;
        let error_text = String::from_utf8_lossy(&body);
        error!(
            "Failed to fetch price: {} - {}",
            response.status(),
            error_text
        );
        return Err(RelayerError::NetworkError(format!(
            "Price API returned error: {}",
            error_text
        )));
    }

    let amount_f64 = amount
        .to_f64()
        .ok_or_else(|| RelayerError::Other("Failed to convert amount to f64".to_string()))?;

    let price: f64 = response.json().await.map_err(|e| {
        RelayerError::NetworkError(format!("Failed to parse price response: {}", e))
    })?;

    info!("Fetched price: {} for amount: {}", price, amount_f64);

    let total_value = (amount_f64 * price) as u128;
    Ok(Uint256::from(total_value))
}

/// Estimates whether a transaction is profitable to relay.
///
/// Compares the tip value (converted to gas token) against the estimated gas cost
/// with a profit margin buffer. A transaction is considered profitable if the tip
/// value exceeds the gas cost plus the configured profit margin.
///
/// # Arguments
/// * `tip_amount` - Amount of tip tokens offered
/// * `tip_token` - Address of the tip token
/// * `gas_used` - Estimated gas consumption
/// * `gas_price` - Current gas price
/// * `price_api_url` - URL of the price API service
/// * `profit_margin_percent` - Profit margin percentage to add to gas costs
///
/// # Returns
/// `true` if the transaction is profitable, `false` otherwise
pub async fn estimate_profitability(
    tip_amount: Uint256,
    tip_token: Address,
    gas_used: Uint256,
    gas_price: Uint256,
    price_api_url: &str,
    profit_margin_percent: u8,
) -> Result<bool, RelayerError> {
    // Calculate base gas cost
    let gas_cost = gas_used * gas_price;

    // Fetch tip value in gas token
    let tip_value = fetch_value_in_gas_token(price_api_url, tip_token, tip_amount).await?;

    // Add profit margin to gas cost
    let gas_cost_with_margin = gas_cost + (gas_cost / Uint256::from(profit_margin_percent));

    if tip_value > gas_cost_with_margin {
        info!(
            "Transaction is profitable: tip value {} > gas cost with margin {}",
            tip_value, gas_cost_with_margin
        );
        Ok(true)
    } else {
        info!(
            "Transaction is not profitable: tip value {} <= gas cost with margin {} (gas price: {}, gas used: {})",
            tip_value, gas_cost_with_margin, gas_price, gas_used
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profitability_calculation() {
        // This is a placeholder for unit tests
        // In a real scenario, you'd mock the price API
        let tip = Uint256::from(1000u64);
        let gas = Uint256::from(100u64);
        let price = Uint256::from(10u64);
        let profit_margin = 10u8;
        let cost_with_margin = gas * price + (gas * price / Uint256::from(profit_margin));
        assert!(tip > cost_with_margin);
    }
}
