use serde_json::{json, Value};
use std::env;
use std::sync::{Arc, RwLock};
use systemet::{Product, Systemet};
use tera::{Context, Tera};
use warp::{reply::html, Filter};

const TEMPLATE_GLOB: &str = "templates/*";
const TEMPLATE: &str = "apk.html";
const KEY_ENV_VAR: &str = "APK_API_KEY";
const PORT_ENV_VAR: &str = "APK_PORT";
const ADDR_ENV_VAR: &str = "APK_ADDR";
const DEFAULT_PORT: u16 = 3030;
const DEFAULT_ADDR: [u8; 4] = [127, 0, 0, 1];
/// In seconds
const UPDATE_INTERVAL: u64 = 7200;
const RETRY_INTERVAL: u64 = 5;

async fn render(tera: &Tera, systemet: &Systemet) -> Result<String, Box<dyn std::error::Error>> {
    eprintln!("Fetching list of products...");
    let products = systemet.get_all_products().await?;

    eprintln!("Categorizing products...");
    let mut wines = Vec::new();
    let mut beers = Vec::new();
    let mut ciders = Vec::new();
    let mut liquors = Vec::new();
    let mut others = Vec::new();

    products
        .into_iter()
        .filter(|drink| {
            drink.alcohol_percentage > 0.0
                && !drink.assortment.as_ref().unwrap().eq("BS")
                && !drink.assortment.as_ref().unwrap().eq("TSLS")
                && !drink.is_completely_out_of_stock
        })
        .for_each(
            |drink| match &drink.category.as_ref().unwrap_or(&"Other".to_string()) as &str {
                "Röda viner" | "Vita viner" | "Mousserande viner" | "Roséviner"
                | "Aperitif & dessert" => wines.push(drink),
                "Öl" => beers.push(drink),
                "Cider och blanddrycker" => {
                    match &drink.sub_category.as_ref().unwrap_or(&"Other".to_string()) as &str {
                        "Cider" => ciders.push(drink),
                        _ => others.push(drink),
                    }
                }
                "Sprit" => liquors.push(drink),
                _ => others.push(drink),
            },
        );
    eprintln!("Sorting...");
    wines.sort_by(apk_comparator);
    beers.sort_by(apk_comparator);
    ciders.sort_by(apk_comparator);
    liquors.sort_by(apk_comparator);
    others.sort_by(apk_comparator);
    let drinks = json!({
        "Öl": beers,
        "Vin": wines,
        "Cider": ciders,
        "Sprit": liquors,
        "Annat": others,
    });
    eprintln!("Rendering...");
    let mut context = Context::new();
    context.insert("drinks", &drinks);
    match tera.render(TEMPLATE, &context) {
        Ok(body) => Ok(body),
        Err(err) => {
            eprintln!("{:?}", err);
            Err(Box::new(err))
        }
    }
}

pub fn format_float(
    value: &serde_json::Value,
    args: &std::collections::HashMap<String, Value>,
) -> tera::Result<serde_json::Value> {
    let number: f64 = serde_json::from_value(value.clone())?;
    let precision = serde_json::from_value(args.get("precision").unwrap().to_owned())?;
    Ok(serde_json::to_value(format!("{:.*}", precision, number))?)
}

pub fn apk_filter(
    value: &serde_json::Value,
    _: &std::collections::HashMap<String, Value>,
) -> tera::Result<serde_json::Value> {
    let drink: Product = serde_json::from_value(value.clone())?;
    Ok(serde_json::to_value(apk(&drink))?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = env::var(KEY_ENV_VAR)?;
    let systemet = Systemet::new(key);
    let mut tera = Tera::new(TEMPLATE_GLOB)?;
    tera.register_filter("apk", apk_filter);
    tera.register_filter("format_float", format_float);
    let page = Arc::new(RwLock::new("".to_string()));
    let page2 = page.clone();

    tokio::spawn(async move {
        let page = page.clone();
        let systemet = systemet.clone();
        loop {
            let delay;
            eprintln!("Updating APK list...");
            match render(&tera, &systemet).await {
                Ok(body) => {
                    let mut page = page.write().unwrap();
                    *page = body;
                    delay = UPDATE_INTERVAL;
                    eprintln!("Succesfully updated APK list");
                }
                Err(err) => {
                    delay = RETRY_INTERVAL;
                    eprintln!("{:?}", err);
                }
            }
            tokio::time::delay_for(std::time::Duration::new(delay, 0)).await;
        }
    });

    let routes = warp::get()
        .map(move || page2.clone())
        .map(|page: Arc<RwLock<String>>| html((*page).read().unwrap().to_string()));

    let port = env::var(PORT_ENV_VAR)
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let addr = env::var(ADDR_ENV_VAR)
        .ok()
        .and_then(|a| a.parse().ok())
        .unwrap_or(DEFAULT_ADDR.into());
    let sock_addr = std::net::SocketAddr::new(addr, port);
    println!("Listening on {}...", sock_addr);
    Ok(warp::serve(routes).run(sock_addr).await)
}

fn apk(drink: &Product) -> f64 {
    drink.alcohol_percentage * drink.volume / (drink.price + drink.recycle_fee)
}

fn basen_apk(drink: &Product) -> f64 {
    basen_price(drink) * drink.volume / (drink.price + drink.recycle_fee)
}

fn basen_price(drink: &Product) -> f64 {
    (drink.price * 1.25 / 5.0).ceil() * 5.0
}

fn apk_comparator(d1: &Product, d2: &Product) -> std::cmp::Ordering {
    if apk(d1) < apk(d2) {
        std::cmp::Ordering::Greater
    } else if apk(d1) > apk(d2) {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Equal
    }
}

//async fn make_list(
//    tera: Tera,
//    drinks: Arc<RwLock<Drinks>>,
//) -> Result<impl warp::Reply, warp::Rejection> {
//    let mut context = Context::new();
//    context.insert("drinks", &drinks.read().unwrap().beers);
//    match tera.render(TEMPLATE, &context) {
//        Ok(body) => Ok(Response::builder().body(body)),
//        Err(err) => {
//            eprintln!("{:?}", err);
//            Ok(err.to_string())
//        }
//    }
//}
