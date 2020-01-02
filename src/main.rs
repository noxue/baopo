use structopt::StructOpt;
use threadpool::ThreadPool;
use std::sync::mpsc::sync_channel;
use std::thread::sleep;
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::io::Read;
use base64::encode;
use serde::{Serialize, Deserialize};
use regex::Regex;
use log::{info, warn, error};
use std::collections::HashMap;
use std::borrow::Borrow;


#[derive(Debug, Clone)]
struct Proxy {
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

//    fn get_proxy(self) -> &'static Proxy {
//        let mut ps = self.proxies;
//        for p in ps.values_mut() {
//            if p.times < self.max_per_proxy_threads as u32 {
//                p.times += 1;
//                return p;
//            }
//        }
//
//        let p = get_proxy();
//        let p = Proxy::new(p);
//        let key = format!("{}", ps.len()).as_str();
//        ps.insert(key.clone(), p.clone());
//        let t = ps.get(key.clone()).unwrap();
//    }
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
    #[structopt(short = "p", long = "per-proxy-threads", required = false, default_value = "10", help = "每个代理允许执行的线程数")]
    pub per_proxy_threads: usize,
}

impl Proxy {
    pub fn new(proxy: reqwest::Proxy) -> Proxy {
        Proxy {
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

    for id in 0..args.threads {
        let receiver = Arc::clone(&receiver);
        let proxies = Arc::clone(&proxies);
        pool.execute(move || {
            loop {
                let mut account = receiver.lock().unwrap().recv().unwrap();
                let mut proxies = proxies.lock().unwrap();
//                let proxy = proxies.get_proxy();


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
                    proxies.proxies.insert(key, v.clone());

                    let result = check_account(account, v);
                    continue;
                }

                let p = p.unwrap();

                let result = check_account(account, p.to_owned());
                p.times -= 1;

            }
        });
    }

    let mut app_code = args.app_code;
    loop {
        for finish_code in args.finish_code..args.finish_code_end {
            sender.send(Account {
                app_code,
                finish_code,
                success: false,
                content: "".to_string(),
                count: args.per_proxy_threads,
            }).unwrap();
        }
        app_code += 1;
    }

    Ok(())
}

fn get_proxy() -> reqwest::Proxy {
    let url = "http://dps.kdlapi.com/api/getdps/?orderid=947758955318965&num=1&pt=1&sep=1";
//    let body = reqwest::get(url).unwrap().text().unwrap();
    let body = "hahaha";
    return reqwest::Proxy::all(format!("http://{}", body).as_str()).unwrap()
        .basic_auth("yes", "61e2a8r9");
}

fn check_account(account: Account, proxy: Proxy) -> bool {
    let mut account = account;
    let client = reqwest::Client::builder()
//        .proxy(proxy.proxy)
        .cookie_store(true)
        .build().unwrap();


    let res = match client.get("https://www.chinadegrees.cn/cqva/gateway.html").send() {
        Ok(v) => v,
        Err(e) => {
            error!("{}", e);
            return false;
        }
    };

    let mut res = match client.get(format!("https://www.chinadegrees.cn/cqva/captcha.html?{}{}", account.app_code, account.finish_code).as_str()).send() {
        Ok(v) => v,
        Err(e) => {
            error!("{}", e);
            return false;
        }
    };

    let mut data = vec![];
    let size = res.read_to_end(&mut data).unwrap();

    let captcha = get_captcha(data).unwrap();

    let old = account.app_code < 2014000000;
    let mut result_url = String::new();
    if old {
        result_url = format!("https://www.chinadegrees.cn/cqva/report/rznb/report-rznb.html?appcod={}&finishcod={:010}&captcha={}&_r=12346", account.app_code.clone(), account.finish_code.clone(), captcha.message.clone());
    } else {
        result_url = format!("http://www.chinadegrees.cn/cqva/report/result.html?appcod={}&finishcod={}&captcha={}&_r=321648", account.app_code.clone(), account.finish_code.clone(), captcha.message.clone());
    }

    let mut res = match client.get(result_url.clone().as_str())
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/77.0.3865.90 Safari/537.36")
        .header("Referer", "https://www.chinadegrees.cn/cqva/gateway.html").send() {
        Ok(v) => v,
        Err(e) => {
            error!("{}", e);
            return false;
        }
    };

    let res = res.text().unwrap();

    if res.find("没有查询到该报告相关信息!").is_some() || res.find("发生错误，请稍后重试").is_some() {
        return false;
    }

    if old {
        account.success = res.find("alert-success").is_some();
        let rg = Regex::new(r#"<div id="textarea2".*?>([\s\S]*?)<center"#).unwrap();
        match rg.captures(res.as_str()) {
            Some(v) => {
                account.content = format!("{}", v.get(1).map_or("", |m| m.as_str()));
            }
            _ => {
                return false;
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
                return false;
            }
        }
    }

    if !account.success {
        return false;
    }

    println!("{:#?}", account);
    true
}

fn get_captcha(data: Vec<u8>) -> Option<Captcha> {
    let data = encode(data.as_slice());

    let mut json = String::from(r#"{"image":""#); //format!(r#"{"image":"{}"}"#, data);
    json.push_str(data.as_str());
    json.push_str(r#""}"#);

    let client = reqwest::Client::new();
    let mut res = client.post("http://106.13.192.37:19952/captcha/v1")
        .header("Content-type", "application/json")
        .body(json).send().unwrap();

    let res = res.text().unwrap();
    let captcha = serde_json::from_str(&res);
    if let Ok(captcha) = captcha {
        return Some(captcha);
    }
    None
}
