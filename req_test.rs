use reqwest::Client;

#[tokio::main]
async fn main() {
    let url = "https://arquivos.receitafederal.gov.br/public.php/dav/files/YggdBLfdninEJX9/2026-02/Estabelecimentos0.zip";
    let client = Client::new();
    let resp = client.head(url).send().await.unwrap();
    println!("Status: {}", resp.status());
    println!("Content-Length: {:?}", resp.content_length());
    println!("Headers: {:?}", resp.headers());
}
