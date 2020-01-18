use base64::encode;
use regex::Regex;
use reqwest::Error as ReqwestError;
use serde::export::fmt::Error;
use serde::export::Formatter;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use structopt::StructOpt;
use threadpool::ThreadPool;

#[derive(Debug, Clone)]
struct Proxy {
    pub key: String,
    pub proxy: reqwest::Proxy,
    // 当前代理被使用的次数
    pub times: usize,
    // 是否过期
    pub expired: bool,
}

#[derive(Debug, Clone)]
struct ProxyList {
    pub proxies: HashMap<String, Proxy>,
    pub max_per_proxy_threads: usize,
}

impl ProxyList {
    fn new(count: usize) -> ProxyList {
        ProxyList {
            proxies: HashMap::new(),
            max_per_proxy_threads: count,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct Captcha {
    pub message: String,
    pub code: i32,
    pub success: bool,
}

#[derive(Debug)]
struct Account {
    pub app_code: u64,
    pub finish_code: u64,
    pub success: bool,
    pub content: String,
    pub count: usize,
}

#[derive(StructOpt, Debug)]
struct Cli {
    #[structopt(short = "a", long = "app-code", help = "申请编号")]
    pub app_code: u64,
    #[structopt(short = "f", long = "finish-code", help = "报告开始编号")]
    pub finish_code: u64,
    #[structopt(short = "e", long = "finish-code-end", help = "报告结束编号")]
    pub finish_code_end: u64,
    #[structopt(short = "t", long = "threads", help = "线程数")]
    pub threads: usize,
    #[structopt(
        short = "p",
        long = "per-proxy-threads",
        required = false,
        default_value = "10",
        help = "每个代理允许执行的线程数"
    )]
    pub per_proxy_threads: usize,
}

impl Proxy {
    pub fn new(proxy: reqwest::Proxy) -> Proxy {
        Proxy {
            key: "".to_string(),
            proxy,
            times: 0,
            expired: false,
        }
    }
}

fn main() -> Result<(), reqwest::Error> {
    let args = Cli::from_args();
    println!("{:#?}", args);

    let proxies = Arc::new(Mutex::new(ProxyList::new(args.per_proxy_threads)));

    let pool = ThreadPool::new(args.threads);
    let (sender, receiver) = sync_channel(1);
    let receiver = Arc::new(Mutex::new(receiver));

    for _ in 0..args.threads {
        let receiver = Arc::clone(&receiver);
        let proxies = Arc::clone(&proxies);
        pool.execute(move || loop {
            let account = receiver.lock().unwrap().recv().unwrap();
            let mut proxies = proxies.lock().unwrap();
            let mut p = None;
            for v in proxies.proxies.values_mut() {
                if v.times < 10 {
                    v.times += 1;
                    p = Some(v);
                    break;
                }
            }
            if p.is_none() {
                let v = get_proxy();
                let mut v = Proxy::new(v.clone());
                let key = format!("{}", proxies.proxies.len());
                v.key = key.clone();
                proxies.proxies.insert(key.clone(), v.clone());

                match check_account(account, v) {
                    Ok(v) => v,
                    Err(e) => match e.kind() {
                        CheckErrorKind::ProxyExpire => {
                            proxies.proxies.remove(key.clone().as_str());
                            false
                        }
                        _ => false,
                    },
                };

                continue;
            }

            let p = p.unwrap();
            p.times -= 1;
            match check_account(account, p.to_owned()) {
                Ok(v) => v,
                Err(e) => match e.kind() {
                    CheckErrorKind::ProxyExpire => {
                        let key = p.key.clone();
                        proxies.proxies.remove(&key);
                        false
                    }
                    _ => false,
                },
            };
        });
    }

    let mut app_code = args.app_code;
    loop {
        for finish_code in args.finish_code..args.finish_code_end {
            sender
                .send(Account {
                    app_code,
                    finish_code,
                    success: false,
                    content: "".to_string(),
                    count: args.per_proxy_threads,
                })
                .unwrap();
        }
        app_code += 1;
    }
}

fn get_proxy() -> reqwest::Proxy {
    let url = "http://dps.kdlapi.com/api/getdps/?orderid=947758955318965&num=1&pt=1&sep=1";
    let body = reqwest::get(url).unwrap().text().unwrap();
    return reqwest::Proxy::http(format!("http://{}", body).as_str()).unwrap();
}

#[derive(Debug)]
struct CheckError {
    v: String,
}

impl std::error::Error for CheckError {
    fn description(&self) -> &str {
        &self.v
    }
}

impl CheckError {
    pub fn kind(&self) -> CheckErrorKind {
        match self.v.as_str() {
            "proxy expire" => CheckErrorKind::ProxyExpire,
            "captcha error" => CheckErrorKind::CaptchaError,
            _ => CheckErrorKind::Other,
        }
    }
}

enum CheckErrorKind {
    ProxyExpire,
    CaptchaError,
    Other,
}

impl CheckErrorKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match *self {
            CheckErrorKind::ProxyExpire => "proxy expire",
            CheckErrorKind::CaptchaError => "captcha error",
            _ => "other",
        }
    }
}

impl From<CheckErrorKind> for CheckError {
    fn from(kind: CheckErrorKind) -> Self {
        CheckError {
            v: kind.as_str().to_string(),
        }
    }
}

impl From<ReqwestError> for CheckError {
    fn from(kind: ReqwestError) -> Self {
        CheckError {
            v: kind.to_string(),
        }
    }
}

impl From<std::io::Error> for CheckError {
    fn from(kind: std::io::Error) -> Self {
        CheckError {
            v: kind.to_string(),
        }
    }
}

impl From<String> for CheckError {
    fn from(v: String) -> Self {
        CheckError { v }
    }
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, _: &mut Formatter<'_>) -> Result<(), Error> {
        Ok(())
    }
}

fn check_account(account: Account, proxy: Proxy) -> Result<bool, CheckError> {
    let proxy = proxy.proxy.basic_auth("yes", "61e2a8r9");
    println!("{:#?}", proxy.clone());

    let mut account = account;
    let client = reqwest::Client::builder()
        .proxy(proxy)
        .cookie_store(true)
        .build()?;

    client
        .get("https://www.chinadegrees.cn/cqva/gateway.html")
        .send()?;

    

    let mut res = client
        .get(
            format!(
                "https://www.chinadegrees.cn/cqva/captcha.html?{}{}",
                account.app_code, account.finish_code
            )
            .as_str(),
        )
        .send()?;

    let mut data = vec![];
    res.read_to_end(&mut data)?;

    let captcha = get_captcha(data)?;

    let old = account.app_code < 2014000000;
    let result_url;
    if old {
        result_url = format!("https://www.chinadegrees.cn/cqva/report/rznb/report-rznb.html?appcod={}&finishcod={:010}&captcha={}&_r=12346", account.app_code.clone(), account.finish_code.clone(), captcha.message.clone());
    } else {
        result_url = format!("http://www.chinadegrees.cn/cqva/report/result.html?appcod={}&finishcod={}&captcha={}&_r=321648", account.app_code.clone(), account.finish_code.clone(), captcha.message.clone());
    }

    let mut res = client.get(result_url.as_str())
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/77.0.3865.90 Safari/537.36")
        .header("Referer", "https://www.chinadegrees.cn/cqva/gateway.html").send()?;

    let res = res.text()?;
    println!("{:?}", res);
    if res.find("没有查询到该报告相关信息!").is_some() || res.find("发生错误，请稍后重试").is_some()
    {
        return Ok(false);
    }

    if old {
        account.success = res.find("alert-success").is_some();
        let rg = Regex::new(r#"<div id="textarea2".*?>([\s\S]*?)<center"#).unwrap();
        match rg.captures(res.as_str()) {
            Some(v) => {
                account.content = format!("{}", v.get(1).map_or("", |m| m.as_str()));
            }
            _ => {
                return Ok(false);
            }
        }
    } else {
        account.success = res.find("reportNumArea").is_some();
        let rg = Regex::new(r#"<div class="reportContent">([\s\S]*?)<p align="center""#).unwrap();
        match rg.captures(res.as_str()) {
            Some(v) => {
                account.content = format!("{}", v.get(1).map_or("", |m| m.as_str()));
            }
            _ => {
                return Ok(false);
            }
        }
    }

    if !account.success {
        return Ok(false);
    }

    println!("=========================================");
    Ok(true)
}

fn get_captcha(data: Vec<u8>) -> Result<Captcha, String> {
    let data = encode(data.as_slice());

    let mut json = String::from(r#"{"image":""#);
    json.push_str(data.as_str());
    json.push_str(r#""}"#);

    let client = reqwest::Client::new();
    let mut res = client
        .post("http://106.13.192.37:19952/captcha/v1")
        .header("Content-type", "application/json")
        .body(json)
        .send()
        .unwrap();

    let res = res.text().unwrap();
    let captcha = serde_json::from_str(&res);
    if let Ok(captcha) = captcha {
        return Ok(captcha);
    }
    Err("error".to_string())
}
