use std::collections::HashMap;
use api::{
    instruments_service_client::InstrumentsServiceClient,
    operations_service_client::OperationsServiceClient, portfolio_request::CurrencyRequest,
    users_service_client::UsersServiceClient, GetAccountsRequest, InstrumentStatus,
    InstrumentsRequest, MoneyValue, PortfolioRequest, Quotation,
};
use serde::Deserialize;
use tonic::{
    metadata::{Ascii, MetadataValue},
    transport::{Channel, ClientTlsConfig},
    Request,
};

#[derive(Deserialize, Debug)]
struct Config {
    assets: Vec<Asset>,
}

#[derive(Deserialize, Debug)]
struct Asset {
    allocation: u8,
    ticker: String,
}

fn interceptor_fn<'a>(
    token: &'a MetadataValue<Ascii>,
) -> impl FnMut(Request<()>) -> tonic::Result<Request<()>, tonic::Status> + 'a {
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

    let tickers = config.assets.iter().map(|e| &e.ticker).collect::<Vec<_>>();
    let mut tickers_ids = vec![];
    for t in tickers {
        let etf = etfs_resp
            .instruments
            .iter()
            .find(|e| e.ticker == *t)
            .unwrap();
        tickers_ids.push(etf.clone());
    }

    let tickers_map = tickers_ids
        .into_iter()
        .map(|e| (e.uid.clone(), e))
        .collect::<HashMap<_, _>>();

    let etfs = portfolio_resp
        .positions
        .into_iter()
        .filter(|e| tickers_map.contains_key(&e.instrument_uid))
        .collect::<Vec<_>>();

    let etfs = tickers_map
        .into_iter()
        .map(|(k, v)| {
            let port_etf = etfs.iter().find(|e| e.instrument_uid == v.uid).unwrap();
            // let etf = etfs_resp
            //     .instruments
            //     .iter()
            //     .find(|e| e.uid == v.uid)
            //     .unwrap();
            let asset = config.assets.iter().find(|e| e.ticker == v.ticker).unwrap();
            (k, (v, port_etf.clone(), asset.allocation))
        })
        .collect::<HashMap<_, _>>();

    // println!("etfs {:?}", etfs);

    let total_etf_value = etfs
        .values()
        .map(|e| {
            (
                e.1.current_price.as_ref().unwrap(),
                e.1.quantity.as_ref().unwrap(),
                e.0.lot,
            )
        })
        .fold(0_f64, |acc, (x, q, lot)| {
            acc + to_float(x) * to_float_quant(q) * (lot as f64)
        });

    println!("total etf value {:?}", total_etf_value);

    let etfs = etfs
        .into_iter()
        .map(|(k, (etf, pos, alloc))| {
            let current_alloc = to_float(pos.current_price.as_ref().unwrap())
                * to_float_quant(pos.quantity.as_ref().unwrap())
                * (etf.lot as f64)
                / total_etf_value
                * 100.0;
            // let current_alloc = current_alloc.round() as u8;
            let shifted = if current_alloc < 20.0 {
                let shift_val = current_alloc * 0.05;
                (current_alloc - alloc as f64).abs() > shift_val
            } else {
                alloc.abs_diff(current_alloc as u8) >= 5
            };

            let vol = current_alloc * total_etf_value / 100.0;

            (k, (etf, pos, alloc, current_alloc, shifted, vol))
        })
        .collect::<HashMap<_, _>>();

    for v in etfs.values() {
        println!(
            "ticker {} alloc {} current {} shifted {} vol {}",
            v.0.ticker, v.2, v.3, v.4, v.5
        );
    }

    let realloc = etfs.values().any(|e| e.4);
    if realloc {
        println!("required changes:");

        for v in etfs.values() {
            let target_vol = (v.2 as f64) * total_etf_value / 100.0;
            let diff = target_vol - v.5;
            let diff_lot = diff / to_float(v.1.current_price.as_ref().unwrap());
            let diff_lot = diff_lot.round() as i32;
            println!(
                "ticker {} target_vol {} lot_change {} current_price {}",
                v.0.ticker,
                target_vol,
                diff_lot,
                to_float(v.1.current_price.as_ref().unwrap()),
            );
        }
    }

    Ok(())
}
