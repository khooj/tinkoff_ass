use anyhow::Result;
use api::{
    operations_service_client::OperationsServiceClient,
    operations_stream_service_client::OperationsStreamServiceClient,
    portfolio_request::CurrencyRequest, users_service_client::UsersServiceClient, Currency,
    GetAccountsRequest, GetAccountsResponse, PortfolioRequest, PortfolioStreamRequest,
};
use secrets::{Secret, SecretBox, SecretVec};
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

fn interceptor_fn<'a>(
    token: &'a MetadataValue<Ascii>,
) -> impl FnMut(Request<()>) -> tonic::Result<Request<()>, tonic::Status> + 'a {
    move |mut req: Request<()>| {
        req.metadata_mut().insert("authorization", token.clone());
        Ok(req)
    }
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
    let mut operations_stream_client =
        OperationsStreamServiceClient::with_interceptor(channel.clone(), interceptor_fn(&token));

    let request = Request::new(GetAccountsRequest {});

    let resp = user_client.get_accounts(request).await?;
    let resp = resp.into_inner();
    println!("accounts: {:?}", resp.accounts);

    let account_id = resp.accounts[0].id.clone();

    let port_stream = operations_stream_client
        .portfolio_stream(PortfolioStreamRequest {
            accounts: vec![account_id.clone()],
        })
        .await?;

    let mut port_stream = port_stream.into_inner();

    let portfolio_resp = operations_client
        .get_portfolio(PortfolioRequest {
            account_id: account_id.clone(),
            currency: Some(CurrencyRequest::Rub.into()),
        })
        .await?;
    println!("portfolio resp: {:?}", portfolio_resp);

    while let Some(r) = port_stream.next().await {
        println!("port stream elem: {:?}", r.unwrap().payload);
    }

    Ok(())
}
