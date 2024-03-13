pub mod tinkoff_api {
    tonic::include_proto!("tinkoff.public.invest.api.contract.v1");
}

use tinkoff_api::{users_service_client::UsersServiceClient, GetAccountsRequest};

use anyhow::Result;
use secrets::{Secret, SecretBox, SecretVec};
use std::io::Read;
use tonic::{
    metadata::MetadataValue,
    transport::{Certificate, Channel, ClientTlsConfig},
    Request,
};
use tracing::instrument;

async fn get_user_accounts(
    mut client: &mut UsersServiceClient<impl tonic::service::Interceptor>,
) -> Result<()> {
    // let request = Request::new(GetAccountsRequest {});

    // let resp = client.get_accounts(request).await?;
    // let resp = resp.into_inner();
    // println!("accounts: {:?}", resp.accounts);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv::dotenv().unwrap();

    // let token = SecretBox::<[u8; 88]>::new(|mut sec| {
    //     let s = std::env::var("TINKOFF_TOKEN").unwrap();
    //     let mut s = std::io::Cursor::new(s.as_bytes());
    //     s.read_exact(&mut sec[..]).unwrap();
    // });

    let token: MetadataValue<_> =
        format!("Bearer {}", std::env::var("TINKOFF_TOKEN").unwrap()).parse()?;

    let tls_config = ClientTlsConfig::new();

    let channel = Channel::from_static("https://sandbox-invest-public-api.tinkoff.ru:443")
        .tls_config(tls_config)?
        .connect()
        .await?;
    println!("connected");
    // let mut client = UsersServiceClient::new(channel);
    let mut client = UsersServiceClient::with_interceptor(channel, move |mut req: Request<()>| {
        req.metadata_mut().insert("authorization", token.clone());
        Ok(req)
    });
    println!("connected");
    let request = Request::new(GetAccountsRequest {});

    let resp = client.get_accounts(request).await?;
    let resp = resp.into_inner();
    println!("accounts: {:?}", resp.accounts);

    Ok(())
}
