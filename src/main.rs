use api::{
    instruments_service_client::InstrumentsServiceClient,
    operations_service_client::OperationsServiceClient, portfolio_request::CurrencyRequest,
    users_service_client::UsersServiceClient, Etf, GetAccountsRequest, InstrumentStatus,
    InstrumentsRequest, MoneyValue, PortfolioPosition, PortfolioRequest, Quotation,
};
use serde::Deserialize;
use std::collections::HashMap;
use tonic::{
    metadata::{Ascii, MetadataValue},
    transport::{Channel, ClientTlsConfig},
    Request,
};

#[derive(Deserialize, Debug)]
struct Config {
    assets: Vec<Asset>,
    change: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct Asset {
    allocation: u8,
    ticker: Ticker,
}

fn interceptor_fn(
    token: &MetadataValue<Ascii>,
) -> impl FnMut(Request<()>) -> tonic::Result<Request<()>, tonic::Status> + '_ {
    move |mut req: Request<()>| {
        req.metadata_mut().insert("authorization", token.clone());
        Ok(req)
    }
}

fn to_float(x: &MoneyValue) -> f64 {
    assert_eq!(x.currency, "rub");
    let nano = x.nano as f64;
    let units = x.units as f64;
    units + nano / 10.0_f64.powi(9)
}

fn to_float_quant(x: &Quotation) -> f64 {
    x.units as f64
}

#[derive(PartialEq, Eq, PartialOrd, Hash)]
struct TinkoffUid(String);

#[derive(PartialEq, Eq, PartialOrd, Hash, Deserialize, Debug, Clone)]
struct Ticker(String);

struct EtfInfo {
    ticker: Ticker,
    target_allocation: f64,
    current_allocation: f64,
    current_price: f64,
    shifted: bool,
    volume: f64,
}

fn calculate_total_etf_value<'a, T>(
    portfolio_etfs: T,
    tinkoff_etfs: &HashMap<TinkoffUid, Etf>,
) -> f64
where
    T: IntoIterator<Item = &'a PortfolioPosition>,
{
    portfolio_etfs
        .into_iter()
        .map(|etf| {
            (
                etf.current_price.clone().unwrap(),
                etf.quantity.clone().unwrap(),
                tinkoff_etfs[&TinkoffUid(etf.instrument_uid.clone())].lot,
            )
        })
        .fold(0_f64, |acc, (x, q, lot)| {
            acc + to_float(&x) * to_float_quant(&q) * (lot as f64)
        })
}

fn calculate_current_etf_allocation_and_deviations<'a, T>(
    tickers: T,
    ticker_to_uid: &HashMap<Ticker, TinkoffUid>,
    portfolio_etfs_data: &HashMap<TinkoffUid, PortfolioPosition>,
    tinkoff_etfs_data: &HashMap<TinkoffUid, Etf>,
    total_etf_value: f64,
    config: &Config,
) -> Vec<EtfInfo>
where
    T: Iterator<Item = &'a Ticker>,
{
    tickers
        .map(|t| {
            let uid = &ticker_to_uid[t];
            let etf_port = &portfolio_etfs_data[uid];
            let api_data = &tinkoff_etfs_data[uid];

            let current_price = to_float(etf_port.current_price.as_ref().unwrap());
            let current_quantity = to_float_quant(etf_port.quantity.as_ref().unwrap());
            let current_alloc =
                current_price * current_quantity * (api_data.lot as f64) / total_etf_value * 100.0;

            let target_alloc = config
                .assets
                .iter()
                .find(|e| e.ticker == *t)
                .unwrap()
                .allocation;
            let shifted = if current_alloc < 20.0 {
                let shift_val = current_alloc * 0.05;
                (current_alloc - target_alloc as f64).abs() > shift_val
            } else {
                target_alloc.abs_diff(current_alloc as u8) >= 5
            };
            let volume = current_alloc * total_etf_value / 100.0;

            EtfInfo {
                ticker: t.clone(),
                target_allocation: target_alloc as f64,
                current_allocation: current_alloc,
                current_price,
                shifted,
                volume,
            }
        })
        .collect()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().unwrap();

    let config_name = std::env::var("TINKOFF_CONFIG").unwrap();
    let config: Config = serde_yaml::from_reader(std::fs::File::open(config_name)?)?;

    let alloc_sum: u8 = config.assets.iter().map(|e| e.allocation).sum();
    if alloc_sum > 100 {
        eprintln!("alloc sum should not be over 100");
        return Err(anyhow::anyhow!("alloc sum should not be over 100"));
    }

    let token: MetadataValue<_> =
        format!("Bearer {}", std::env::var("MAIN_TINKOFF_TOKEN").unwrap()).parse()?;

    let tls_config = ClientTlsConfig::new();

    let channel = Channel::from_static("https://invest-public-api.tinkoff.ru:443")
        .tls_config(tls_config)?
        .connect()
        .await?;

    let mut user_client =
        UsersServiceClient::with_interceptor(channel.clone(), interceptor_fn(&token));
    let mut operations_client =
        OperationsServiceClient::with_interceptor(channel.clone(), interceptor_fn(&token));
    let mut instruments_client =
        InstrumentsServiceClient::with_interceptor(channel.clone(), interceptor_fn(&token));

    let request = Request::new(GetAccountsRequest {});

    let resp = user_client.get_accounts(request).await?.into_inner();

    let account_id = resp.accounts[0].id.clone();

    let portfolio_resp = operations_client
        .get_portfolio(PortfolioRequest {
            account_id: account_id.clone(),
            currency: Some(CurrencyRequest::Rub.into()),
        })
        .await?
        .into_inner();
    let etfs_resp = instruments_client
        .etfs(InstrumentsRequest {
            instrument_status: Some(InstrumentStatus::All.into()),
        })
        .await?
        .into_inner();

    let tickers = config
        .assets
        .iter()
        .map(|e| e.ticker.clone())
        .collect::<Vec<_>>();
    let tinkoff_etfs_data = tickers
        .iter()
        .map(|t| {
            let etf = etfs_resp
                .instruments
                .iter()
                .find(|e| e.ticker == t.0)
                .unwrap()
                .clone();
            (TinkoffUid(etf.uid.clone()), etf)
        })
        .collect::<HashMap<_, _>>();
    let tinkoff_ticker_to_uid = tickers
        .iter()
        .map(|t| {
            let etf = etfs_resp
                .instruments
                .iter()
                .find(|e| e.ticker == t.0)
                .unwrap()
                .clone();
            (t.clone(), TinkoffUid(etf.uid.clone()))
        })
        .collect::<HashMap<_, _>>();

    let portfolio_etfs_data = portfolio_resp
        .positions
        .into_iter()
        .filter(|e| tinkoff_etfs_data.contains_key(&TinkoffUid(e.instrument_uid.clone())))
        .map(|e| (TinkoffUid(e.instrument_uid.clone()), e))
        .collect::<HashMap<_, _>>();

    let total_etf_value =
        calculate_total_etf_value(portfolio_etfs_data.values(), &tinkoff_etfs_data);

    println!("total etf value {:?}", total_etf_value);

    let target_etf_value = if let Some(change) = config.change {
        let target = total_etf_value + change;
        println!("ATTENTION! CALCULATIONS DONE ACCORDING TO MANUAL VALUE CHANGE!");
        println!("total etf value after change applied {:?}", target);
        target
    } else {
        total_etf_value
    };

    let current_etf_info_converted = calculate_current_etf_allocation_and_deviations(
        tickers.iter(),
        &tinkoff_ticker_to_uid,
        &portfolio_etfs_data,
        &tinkoff_etfs_data,
        target_etf_value,
        &config,
    );

    for v in &current_etf_info_converted {
        println!(
            "ticker {} alloc {} current {} shifted {} vol {}",
            v.ticker.0, v.target_allocation, v.current_allocation, v.shifted, v.volume
        );
    }

    let realloc = current_etf_info_converted.iter().any(|e| e.shifted);
    if realloc {
        println!("required changes:");

        for v in &current_etf_info_converted {
            let target_vol = v.target_allocation * target_etf_value / 100.0;
            let diff = target_vol - v.volume;
            let diff_lot = diff / v.current_price;
            let diff_lot = diff_lot.round() as i32;
            println!(
                "ticker {} target_vol {} lot_change {}: {} current_price {}",
                v.ticker.0, target_vol, if diff_lot < 0 { "SELL" } else { "BUY"}, diff_lot.abs(), v.current_price,
            );
        }
    }

    Ok(())
}
