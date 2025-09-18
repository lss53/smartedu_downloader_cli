// src/main.rs

use chrono::Utc;
use clap::Parser;
use colored::*;
use futures::stream::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::{error, info, warn};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Client;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

// --- 1. 全局常量和静态变量 ---
static SYMBOL_PROMPT: &str = ">";
static SYMBOL_SUCCESS: &str = "[OK]";
static SYMBOL_ERROR: &str = "[X]";
static SYMBOL_INFO: &str = "[i]";
static SYMBOL_WARNING: &str = "[!]";
static SYMBOL_END: &str = "[DONE]";
static SYMBOL_STATS: &str = "---";
static SYMBOL_DIVIDER: &str = "═";
static TOKEN_FILE: &str = ".access_token";
static MAX_RETRIES: u32 = 3;
static RETRY_BASE_DELAY_MS: u64 = 500;

static FILENAME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r#"[<>:"/\\|?*]"#).unwrap());
static UUID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap());

static PROGRESS_STYLE: Lazy<ProgressStyle> = Lazy::new(|| {
    ProgressStyle::default_bar()
        .template("{msg:.cyan}\n{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
        .unwrap()
        .progress_chars("━╸ ")
});

static FINISHED_STYLE: Lazy<ProgressStyle> = Lazy::new(|| {
    ProgressStyle::default_bar()
        .template("{msg}")
        .unwrap()
});


// --- 2. 错误处理 ---
#[derive(thiserror::Error, Debug)]
enum AppError {
    #[error("网络请求错误: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("文件IO错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON解析错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("获取教材详情失败: {0}")]
    DetailFetch(String),
    #[error("无效的输入: {0}")]
    InvalidInput(String),
    #[error("目录创建失败: {0}")]
    DirCreation(String),
}

// --- 3. 数据结构定义 ---
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DownloadStatus {
    Success,
    SuccessNoValidation,
    Skipped,
    TokenError,
    Md5ValidationFailed,
    SizeValidationFailed,
    NetworkError,
    FailGetDetails,
    UnexpectedError,
}

#[derive(Debug)]
struct TextbookInfo {
    download_url: String,
    filename: String,
    expected_md5: Option<String>,
    expected_size: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct TextbookDetailsResponse {
    ti_items: Vec<TechInfoItem>,
    title: String,
}

#[derive(Deserialize, Debug)]
struct TechInfoItem {
    ti_file_flag: String,
    ti_format: String,
    ti_storages: Vec<String>,
    ti_md5: Option<String>,
    ti_size: Option<u64>,
}

// --- 4. 命令行参数定义 ---
#[derive(Parser, Debug, Clone)]
#[command(
    name = "smartedu_downloader",
    author = "ds",
    version = "1.0",
    about = "国家智慧中小学教材下载命令行工具 (Rust版)",
    long_about = None,
    after_help = "示例:\n  # 下载单个URL\n  smartedu_downloader -u \"<教材URL>\" -t \"<你的TOKEN>\"\n\n  # 从文件批量下载并指定输出目录\n  smartedu_downloader -i urls.txt -o ./教材下载 -t \"<你的TOKEN>\""
)]
struct Cli {
    #[arg(short, long, help = "一个或多个教材页面URL", action = clap::ArgAction::Append)]
    url: Vec<String>,
    #[arg(short, long, name = "content_id", help = "一个或多个教材Content ID", action = clap::ArgAction::Append)]
    content_id: Vec<String>,
    #[arg(short, long, help = "包含URL/ID的文本文件路径")]
    input_file: Option<PathBuf>,
    #[arg(short, long, help = "访问令牌(Access Token)")]
    token: Option<String>,
    #[arg(short, long, help = "输出文件路径或目录")]
    output: Option<String>,
    #[arg(short, long, help = "启用详细调试日志")]
    debug: bool,
    #[arg(long, help = "最大并发下载数", default_value_t = 5)]
    max_concurrent_downloads: usize,
}

// --- 5. 核心及辅助功能函数 ---

fn get_content_id(input: &str) -> Option<String> {
    if UUID_REGEX.is_match(input) {
        Some(input.to_string())
    } else if let Ok(url) = reqwest::Url::parse(input) {
        url.query_pairs().find_map(|(key, value)| {
            (key == "contentId" && UUID_REGEX.is_match(&value)).then(|| value.into_owned())
        })
    } else {
        None
    }
}

fn sanitize_filename(filename: &str) -> String {
    FILENAME_REGEX.replace_all(filename, "_").to_string()
}

async fn calculate_file_md5(path: &Path) -> Result<String, io::Error> {
    let mut file = File::open(path).await?;
    let mut context = md5::Context::new();
    let mut buffer = [0; 8192];
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 { break; }
        context.consume(&buffer[..n]);
    }
    Ok(format!("{:x}", context.compute()))
}

async fn get_textbook_details(client: &Client, content_id: &str, access_token: &str) -> Result<TextbookInfo, AppError> {
    let url = format!("https://s-file-2.ykt.cbern.com.cn/zxx/ndrv2/resources/tch_material/details/{}.json", content_id);
    let data = client.get(&url).send().await?.error_for_status()?.json::<TextbookDetailsResponse>().await?;
    let source_item = data.ti_items.iter()
        .find(|item| item.ti_file_flag == "source" && item.ti_format == "pdf")
        .ok_or_else(|| AppError::DetailFetch(format!("在内容ID '{}' 中未找到源PDF文件信息", content_id)))?;
    let pdf_url_base = source_item.ti_storages.first()
        .ok_or_else(|| AppError::DetailFetch(format!("在内容ID '{}' 中未找到PDF下载地址", content_id)))?;
    let is_pdf_pdf = pdf_url_base.to_lowercase().ends_with("pdf.pdf");
    let mut final_filename = if is_pdf_pdf {
        data.title.clone()
    } else {
        Path::new(pdf_url_base)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| content_id.to_string())
    };
    if !final_filename.to_lowercase().ends_with(".pdf") {
        final_filename.push_str(".pdf");
    }
    Ok(TextbookInfo {
        download_url: format!("{}?accessToken={}", pdf_url_base, access_token),
        filename: sanitize_filename(&final_filename),
        expected_md5: if is_pdf_pdf { None } else { source_item.ti_md5.clone() },
        expected_size: source_item.ti_size,
    })
}

async fn validate_local_file(path: &Path, info: &TextbookInfo) -> Result<DownloadStatus, AppError> {
    if !path.exists() { return Ok(DownloadStatus::SizeValidationFailed); }
    if let Some(expected_md5) = &info.expected_md5 {
        if let Ok(actual_md5) = calculate_file_md5(path).await {
            if actual_md5 == *expected_md5 { return Ok(DownloadStatus::Success); }
        }
    }
    if let Some(expected_size) = info.expected_size {
        if let Ok(metadata) = fs::metadata(path).await {
            if metadata.len() == expected_size { return Ok(DownloadStatus::Success); }
        }
    }
    if info.expected_md5.is_none() && info.expected_size.is_none() { return Ok(DownloadStatus::SuccessNoValidation); }
    if info.expected_md5.is_some() { Ok(DownloadStatus::Md5ValidationFailed) } 
    else { Ok(DownloadStatus::SizeValidationFailed) }
}

async fn download_file(client: &Client, info: &TextbookInfo, dest_path: &Path, mp: Arc<MultiProgress>) -> Result<DownloadStatus, AppError> {
    let pb = mp.add(ProgressBar::new(info.expected_size.unwrap_or(0)));
    pb.set_style(PROGRESS_STYLE.clone());
    pb.set_message(info.filename.clone());

    // 将所有可能失败的逻辑放入一个 async 块中
    let result: Result<DownloadStatus, AppError> = async {
        let mut last_error: Option<AppError> = None;
        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let wait_time = Duration::from_millis(RETRY_BASE_DELAY_MS * 2u64.pow(attempt - 1));
                pb.println(format!("{} '{}' 第{}次下载失败, {:.1?}后重试...", SYMBOL_WARNING, info.filename, attempt, wait_time));
                tokio::time::sleep(wait_time).await;
            }
            pb.set_position(0);

            let response_result = client.get(&info.download_url).send().await;

            match response_result {
                Ok(response) => match response.error_for_status() {
                    Ok(resp) => {
                        let mut file = File::create(&dest_path).await?;
                        let mut stream = resp.bytes_stream();
                        while let Some(chunk_result) = stream.next().await {
                            let chunk = chunk_result?;
                            file.write_all(&chunk).await?;
                            pb.inc(chunk.len() as u64);
                        }
                        file.flush().await?;
                        
                        // 下载成功，直接返回校验结果
                        return validate_local_file(dest_path, info).await;
                    }
                    Err(e) => {
                        // HTTP 状态码错误 (e.g., 404, 500)
                        if e.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
                            // 这是个不可重试的致命错误，直接返回
                            return Ok(DownloadStatus::TokenError);
                        }
                        last_error = Some(e.into());
                    }
                },
                Err(e) => {
                    // 网络层错误 (e.g., DNS, TCP)
                    last_error = Some(e.into());
                }
            }
        }
        // 如果循环结束仍然失败，返回最后一次的错误
        Err(last_error.unwrap_or(AppError::DetailFetch("未知下载错误".into())))
    }.await;

    // 在外部统一处理结果，并确保进度条被终结
    pb.set_style(FINISHED_STYLE.clone());
    match result {
        Ok(DownloadStatus::Success) => {
            pb.finish_with_message(format!("{} '{}' {}", SYMBOL_SUCCESS.green(), info.filename, "校验通过".green()));
            Ok(DownloadStatus::Success)
        }
        Ok(DownloadStatus::SuccessNoValidation) => {
            pb.finish_with_message(format!("{} '{}' {}", SYMBOL_WARNING.yellow(), info.filename, "无校验信息".yellow()));
            Ok(DownloadStatus::SuccessNoValidation)
        }
        Ok(DownloadStatus::TokenError) => {
            pb.finish_with_message(format!("{} '{}' {}", SYMBOL_ERROR.red(), info.filename, "Token错误或过期".red()));
            Ok(DownloadStatus::TokenError)
        }
        Ok(status) => { // 其他校验失败的状态
            pb.finish_with_message(format!("{} '{}' {}", SYMBOL_ERROR.red(), info.filename, "校验失败".red()));
            Ok(status)
        }
        Err(e) => { // 所有在 async 块中发生的 I/O 错误或重试耗尽后的网络错误
            pb.finish_with_message(format!("{} '{}' {}: {}", SYMBOL_ERROR.red(), info.filename, "下载失败".red(), e));
            Ok(DownloadStatus::NetworkError) // 将所有最终错误归类为网络错误
        }
    }
}

async fn process_single_task(client: Arc<Client>, args: Arc<Cli>, item_data: (String, String), dest_folder: Arc<PathBuf>, mp: Arc<MultiProgress>) -> (String, String, DownloadStatus) {
    let (content_id, original_input) = item_data;
    let token = match args.token.as_deref() {
        Some(t) => t,
        None => return (original_input, String::new(), DownloadStatus::TokenError)
    };

    let details = match get_textbook_details(&client, &content_id, token).await {
        Ok(d) => d,
        Err(e) => {
            // 对于非下载阶段的错误，使用log打印，不干扰进度条
            error!("{} 获取'{}' (ID: {}) 详情失败: {}", SYMBOL_ERROR, original_input, content_id, e);
            return (original_input, String::new(), DownloadStatus::FailGetDetails);
        }
    };
    let is_batch = args.url.len() + args.content_id.len() > 1 || args.input_file.is_some();
    let final_filename = if !is_batch {
        if let Some(output) = &args.output {
            let output_path = Path::new(output);
            // 检查 output 参数是否看起来像一个文件名
            if !output.ends_with('/') && !output.ends_with('\\') && output_path.file_name().is_some() {
                output_path.file_name().unwrap().to_string_lossy().to_string()
            } else {
                details.filename.clone()
            }
        } else {
            details.filename.clone()
        }
    } else {
        details.filename.clone()
    };
    let full_output_path = dest_folder.join(&final_filename);
    
    if full_output_path.exists() {
        match validate_local_file(&full_output_path, &details).await {
            Ok(DownloadStatus::Success) | Ok(DownloadStatus::SuccessNoValidation) => {
                info!("{} '{}' {}", SYMBOL_SUCCESS.green(), final_filename, "已存在且校验一致, 跳过".dimmed());
                return (original_input, full_output_path.to_string_lossy().to_string(), DownloadStatus::Skipped);
            }
            _ => {
                info!("{} '{}' {}", SYMBOL_WARNING.yellow(), final_filename, "校验不一致, 重新下载".dimmed());
            }
        }
    }

    match download_file(&client, &details, &full_output_path, mp).await {
        Ok(status) => (original_input, final_filename, status),
        Err(e) => {
            error!("下载'{}' (ID: {}) 时发生意外错误: {}", final_filename, content_id, e);
            (original_input, final_filename, DownloadStatus::UnexpectedError)
        }
    }
}

fn print_token_guide() {
    let content = include_str!("ACCESS_TOKEN_GUIDE.txt");
    let lines: Vec<&str> = content.lines().collect();
    let divider = SYMBOL_DIVIDER.repeat(60);
    
    println!("\n{}", divider.blue().bold());
    
    // 打印标题行
    if let Some(first_line) = lines.first() {
        println!("  {}", first_line.bold().yellow());
    }
    
    let mut in_code_block = false;
    
    for line in lines.iter().skip(1) {
        if in_code_block {
            // 代码块结束标记 - 不显示
            if line.starts_with("```") {
                in_code_block = false;
                continue;
            } else {
                // 代码块内容
                println!("  {}", line.cyan());
            }
            continue;
        }
        
        // 检测代码块开始 - 不显示
        if line.starts_with("```javascript") {
            in_code_block = true;
            continue;
        }
        
        // 处理URL链接 - 精确地只给URL部分添加下划线
        if let Some(url_start) = line.find("https://") {
            // 从 URL 开始的部分切片
            let url_and_after = &line[url_start..];
            
            // 查找 URL 的结束位置。这里我们定义为遇到空格或右括号就结束。
            // 如果没找到，就认为 URL 一直持续到行尾。
            let url_end_offset = url_and_after.find(|c| c == ' ' || c == ')')
                .unwrap_or(url_and_after.len());

            // 将行分割成三部分：URL前，URL本身，URL后
            let before_url = &line[..url_start];
            let url_part = &url_and_after[..url_end_offset];
            let after_url = &url_and_after[url_end_offset..];

            println!("  {}{}{}", before_url, url_part.purple().underline(), after_url);
            continue;
        }
        
        // 处理步骤编号
        if line.starts_with(|c: char| c.is_ascii_digit() && line.contains('.')) {
            println!("  {}", line.bold().green());
        } 
        // 处理子项目符号
        else if line.trim_start().starts_with('-') {
            println!("  {}", line.dimmed());
        }
        // 普通文本
        else {
            println!("  {}", line);
        }
    }
    
    println!("{}", divider.blue().bold());
}

fn read_input_file(path: &Path) -> Result<Vec<String>, AppError> {
    let file = std::fs::File::open(path)?;
    Ok(io::BufReader::new(file).lines().filter_map(Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect())
}

async fn handle_token_input(cli: &Cli) -> Result<String, AppError> {
    if let Some(token) = &cli.token { return Ok(token.clone()); }
    if let Ok(token_from_file) = fs::read_to_string(TOKEN_FILE).await {
        print!("{} 检测到已保存的 Token，是否使用？(y/n): ", SYMBOL_PROMPT);
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() && input.trim().to_lowercase().starts_with('y') {
            return Ok(token_from_file.trim().to_string());
        }
    }
    print_token_guide();
    loop {
        print!("{} 请输入您的访问令牌 (Access Token): ", SYMBOL_PROMPT);
        io::stdout().flush()?;
        let mut token_input = String::new();
        io::stdin().read_line(&mut token_input)?;
        let token = token_input.trim();
        if !token.is_empty() {
            if let Err(e) = fs::write(TOKEN_FILE, token).await { warn!("{} 无法保存Token到文件: {}", SYMBOL_WARNING, e); }
            return Ok(token.to_string());
        }
        println!("{} 访问令牌不能为空，请重新输入。", SYMBOL_ERROR);
    }
}

async fn determine_output_dir(cli: &Cli, is_batch: bool) -> Result<PathBuf, AppError> {
    let output_str = cli.output.as_deref().unwrap_or(".");
    let output_path = PathBuf::from(output_str);
    if is_batch && output_path.is_file() {
        return Err(AppError::InvalidInput("批量下载时, '-o' 参数必须是目录, 不能是文件。".into()));
    }
    let dest_folder = if output_str.ends_with('/') || output_str.ends_with('\\') || !output_path.exists() || output_path.is_dir() {
        output_path
    } else {
        output_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()
    };
    fs::create_dir_all(&dest_folder).await.map_err(|e| AppError::DirCreation(e.to_string()))?;
    info!("文件将保存到目录: '{}'", dest_folder.display());
    Ok(dest_folder)
}

fn collect_download_items(cli: &Cli) -> Result<Vec<(String, String)>, AppError> {
    let mut download_items = Vec::new();
    let mut processed_ids = HashSet::new();
    let mut add_unique_item = |original: &str, source: &str| {
        if let Some(id) = get_content_id(original) {
            if processed_ids.insert(id.clone()) {
                info!("{} 已添加: ID {}... (来源: {})", SYMBOL_SUCCESS, &id[..8], source);
                download_items.push((id, original.to_string()));
            } else {
                info!("{} 检测到重复项，已跳过: '{}'", SYMBOL_INFO, original);
            }
        } else {
            warn!("{} 无效输入，已跳过: '{}' (来源: {})", SYMBOL_WARNING, original, source);
        }
    };
    for url in &cli.url { add_unique_item(url, "命令行URL"); }
    for cid in &cli.content_id { add_unique_item(cid, "命令行ID"); }
    if let Some(path) = &cli.input_file {
        for (i, line) in read_input_file(path)?.iter().enumerate() {
            add_unique_item(line, &format!("文件第 {} 行", i + 1));
        }
    }
    if download_items.is_empty() {
        return Err(AppError::InvalidInput("未找到任何有效的下载项。请检查输入。".into()));
    }
    Ok(download_items)
}

fn process_download_results(results: Vec<Result<(String, String, DownloadStatus), tokio::task::JoinError>>) {
    let mut stats = HashMap::new();
    let mut failed_details = Vec::new();
    let mut skipped_details = Vec::new();

    for res in results {
        match res {
            Ok((original, filename, status)) => {
                *stats.entry(status).or_insert(0) += 1;
                match status {
                    DownloadStatus::Skipped => {
                        skipped_details.push(format!("'{}'", filename));
                    }
                    DownloadStatus::Success | DownloadStatus::SuccessNoValidation => {
                        // 成功状态，这里不需要额外操作
                    }
                    _ => { // 捕获所有其他失败状态
                        let reason = format!("{:?}", status);
                        failed_details.push(format!("'{}': {}", original, reason));
                    }
                }
            }
            Err(e) => { 
                *stats.entry(DownloadStatus::UnexpectedError).or_insert(0) += 1;
                failed_details.push(format!("任务执行时发生Panic: {}", e)); 
            }
        }
    }

    let successful_count = stats.get(&DownloadStatus::Success).unwrap_or(&0) + stats.get(&DownloadStatus::SuccessNoValidation).unwrap_or(&0);
    let skipped_count = stats.get(&DownloadStatus::Skipped).unwrap_or(&0);
    let failed_count = failed_details.len();
    let total_tasks = successful_count + skipped_count + failed_count;

    info!("\n{0}\n{1} {2}下载任务完成！", SYMBOL_DIVIDER.repeat(50), SYMBOL_END, if total_tasks > 1 { "批量" } else { "单次" });
    info!("{} 总计: {} | 成功: {} | 失败: {} | 跳过: {}", SYMBOL_STATS, total_tasks, successful_count, failed_count, skipped_count);
    if !skipped_details.is_empty() {
        info!("{} 跳过的文件 (已存在):", SYMBOL_STATS);
        for item in skipped_details { info!("  - {}", item); }
    }
    if !failed_details.is_empty() {
        error!("{} 失败的详情:", SYMBOL_STATS);
        for item in failed_details { error!("  - {}", item); }
    }
    info!("{}", SYMBOL_DIVIDER.repeat(50));
}

// --- 6. 主程序 ---
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let log_level = if cli.debug { "debug" } else { "info" };
    
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(log_level))
        .format(|buf, record| {
            let now = Utc::now();
            let local_time = now.with_timezone(&chrono::Local);
            writeln!(
                buf,
                "{} [{}] - {}",
                local_time.format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .init();

    if cli.url.is_empty() && cli.content_id.is_empty() && cli.input_file.is_none() {
        return Err(AppError::InvalidInput("必须至少提供一个输入源 (-u, -c, 或 -i)".into()).into());
    }
    
    let token = handle_token_input(&cli).await?;
    let mut final_args = cli.clone();
    final_args.token = Some(token);
    let final_args = Arc::new(final_args);
    
    let download_items = collect_download_items(&final_args)?;
    let is_batch = download_items.len() > 1;
    let dest_folder = Arc::new(determine_output_dir(&final_args, is_batch).await?);
    
    let multi_progress = Arc::new(MultiProgress::new());
    let client = Arc::new(Client::new());
    let semaphore = Arc::new(Semaphore::new(final_args.max_concurrent_downloads));
    let mut tasks = Vec::new();
    
    for item in download_items {
        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let args = final_args.clone();
        let dest = dest_folder.clone();
        let mp = multi_progress.clone();
        
        tasks.push(tokio::spawn(async move {
            let result = process_single_task(client, args, item, dest, mp).await;
            drop(permit); // 明确释放信号量许可
            result
        }));
    }
    
    let results = futures::future::join_all(tasks).await;

    process_download_results(results);
    
    Ok(())
}