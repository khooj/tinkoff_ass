use std::{collections::HashMap, panic};

use anyhow::Result;
use api::{
    instruments_service_client::InstrumentsServiceClient,
    operations_service_client::OperationsServiceClient,
    operations_stream_service_client::OperationsStreamServiceClient,
    portfolio_request::CurrencyRequest, users_service_client::UsersServiceClient, Currency,
    FindInstrumentRequest, GetAccountsRequest, GetAccountsResponse, InstrumentIdType,
    InstrumentRequest, InstrumentStatus, InstrumentType, InstrumentsRequest, MoneyValue,
    PortfolioRequest, PortfolioStreamRequest, Quotation,
};
use refinement::{Predicate, Refinement};
use secrets::{Secret, SecretBox, SecretVec};
use serde::{Deserialize, Deserializer};
use tokio_stream::{Stream, StreamExt};
use tonic::{
    metadata::{Ascii, MetadataValue},
    transport::{Certificate, Channel, ClientTlsConfig},
    Request,
};
use tracing::instrument;

// async fn _get_user_accounts<F>(mut client: UsersServiceClient<F>) -> Result<()>
// where
//     F: tonic::client::GrpcService<tonic::body::BoxBody>,
//     F::ResponseBody: tonic::codegen::Body<Data = bytes::Bytes> + Send + 'static,
//     <F::ResponseBody as tonic::codegen::Body>::Error: Into<tonic::codegen::StdError> + Send,
// {
//     let request = Request::new(GetAccountsRequest {});

//     let resp = client.get_accounts(request).await?;
//     let resp = resp.into_inner();
//     println!("accounts: {:?}", resp.accounts);
//     Ok(())
// }

// wont use because of deserialization
struct Percent;

impl Predicate<u8> for Percent {
    fn test(x: &u8) -> bool {
        *x <= 100
    }
}

type PercentU8 = Refinement<u8, Percent>;

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

struct MoneyValueOps(MoneyValue);

impl MoneyValueOps {
    fn try_overflowing_add(mut self, rhs: MoneyValue) -> anyhow::Result<MoneyValueOps> {
        if self.0.currency != rhs.currency {
            return Err(anyhow::anyhow!(
                "currency not the same: {} {}",
                self.0.currency,
                rhs.currency
            ));
        }

        let (v, overflowed) = self.0.nano.overflowing_add(rhs.nano);
        self.0.nano = v;

        let sh = if overflowed { 1 } else { 0 };

        let (v, overflowed) = self.0.units.overflowing_add(rhs.units);
        if overflowed {
            return Err(anyhow::anyhow!("overflowed units"));
        }
        self.0.units = v;
        let (v, overflowed) = self.0.units.overflowing_add(sh as i64);
        if overflowed {
            return Err(anyhow::anyhow!("overflowed units"));
        }
        self.0.units = v;
        Ok(self)
    }

    fn try_multiply(mut self, rhs: Quotation) -> anyhow::Result<MoneyValueOps> {
        panic!("fix multiplication");
        let MoneyValue {
            currency,
            units,
            nano,
        } = self.0;

        let Quotation {
            units: units_q,
            nano: nano_q,
        } = rhs;

        let (nano, of) = nano.overflowing_mul(nano_q);
        let sh = if of { 1 } else { 0 };
        let (units, of) = units.overflowing_mul(units_q);
        if of {
            return Err(anyhow::anyhow!("overflowed multiply"));
        }
        let (units, of) = units.overflowing_add(sh);
        if of {
            return Err(anyhow::anyhow!("overflowed multiply"));
        }
        Ok(MoneyValueOps(MoneyValue {
            currency,
            units,
            nano,
        }))
    }

    fn multiply(self, rhs: i32) -> anyhow::Result<MoneyValueOps> {
        let rhs = Quotation {
            units: rhs as i64,
            nano: 0,
        };
        self.try_multiply(rhs)
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

    // let token = SecretBox::<[u8; 88]>::new(|mut sec| {
    //     let s = std::env::var("TINKOFF_TOKEN").unwrap();
    //     let mut s = std::io::Cursor::new(s.as_bytes());
    //     s.read_exact(&mut sec[..]).unwrap();
    // });

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
    // println!("accounts: {:?}", resp.accounts);

    let account_id = resp.accounts[0].id.clone();

    // let port_stream = operations_stream_client
    //     .portfolio_stream(PortfolioStreamRequest {
    //         accounts: vec![account_id.clone()],
    //     })
    //     .await?;

    // let mut port_stream = port_stream.into_inner();

    let portfolio_resp = operations_client
        .get_portfolio(PortfolioRequest {
            account_id: account_id.clone(),
            currency: Some(CurrencyRequest::Rub.into()),
        })
        .await?
        .into_inner();
    // println!("portfolio resp: {:?}", portfolio_resp);
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
