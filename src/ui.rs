use astrobox_ng_wit::astrobox::psys_host::{
    clipboard, device, dialog, interconnect, register, thirdpartyapp, ui_v3 as ui,
};
use base64::Engine;
use image::{GrayImage, Luma, imageops};
use serde::Serialize;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

const PACKAGE_NAME: &str = "com.xw.mibandtotp";
const SEND_EVENT: &str = "send_totp";
const EDIT_EVENT: &str = "edit_input";
const CLIPBOARD_EVENT: &str = "send_clipboard";
const QR_IMAGE_EVENT: &str = "send_qr_image";
const QUERY_COUNT_EVENT: &str = "query_wear_count";
const BACKUP_WEAR_KEYS_EVENT: &str = "backup_wear_keys";
const INPUT_EVENT: &str = "totp_input";

#[derive(Default)]
struct UiState {
    root_element_id: Option<String>,
    input_text: String,
    status: String,
    busy: bool,
    install_check_started: bool,
    pending_action: Option<String>,
    pending_payload: Option<String>,
}

static UI_STATE: OnceLock<Mutex<UiState>> = OnceLock::new();

fn ui_state() -> &'static Mutex<UiState> {
    UI_STATE.get_or_init(|| Mutex::new(UiState::default()))
}

#[derive(Serialize)]
struct TOTPInfo {
    name: String,
    usr: String,
    key: String,
    algorithm: String,
    digits: u32,
    period: u32,
}

#[derive(Serialize)]
struct PushPayload {
    list: Vec<TOTPInfo>,
}

enum ParseResult {
    Ok(Vec<TOTPInfo>),
    Err(String),
}

pub async fn ui_event_processor(event: ui::Event, event_id: String, _event_payload: String) {
    tracing::info!(
        "ui_event_processor start: event_id={}, event={:?}, payload={}",
        event_id,
        event,
        _event_payload
    );

    match event {
        ui::Event::Input | ui::Event::Change if event_id == INPUT_EVENT => {
            let input = extract_event_value(&_event_payload);
            tracing::info!("input event captured: chars={}", input.chars().count());
            remember_input_text(input);
        }
        ui::Event::PointerUp => {
            tracing::info!(
                "pointer up observed; waiting for click: event_id={}",
                event_id
            );
        }
        ui::Event::Click => match event_id.as_str() {
            SEND_EVENT | EDIT_EVENT | CLIPBOARD_EVENT | QR_IMAGE_EVENT | QUERY_COUNT_EVENT | BACKUP_WEAR_KEYS_EVENT
                if is_busy() =>
            {
                tracing::info!(
                    "click ignored because plugin is busy: event_id={}",
                    event_id
                );
            }
            SEND_EVENT => {
                tracing::info!("dispatch click action: send input");
                set_status("已收到点击事件：推送输入内容".to_string());
                input_and_push().await;
            }
            EDIT_EVENT => {
                tracing::info!("dispatch click action: edit input");
                edit_input().await;
            }
            CLIPBOARD_EVENT => {
                tracing::info!("dispatch click action: clipboard");
                set_status("已收到点击事件：读取剪贴板".to_string());
                clipboard_to_input().await;
            }
            QR_IMAGE_EVENT => {
                tracing::info!("dispatch click action: qr image");
                set_status("已收到点击事件：选择二维码图片".to_string());
                qr_image_to_input().await;
            }
            QUERY_COUNT_EVENT => {
                tracing::info!("dispatch click action: query wear count");
                query_wear_account_count().await;
            }
            BACKUP_WEAR_KEYS_EVENT => {
                tracing::info!("dispatch click action: backup wear keys");
                backup_wear_keys().await;
            }
            _ => {
                tracing::warn!("unknown click event_id={}", event_id);
            }
        },
        _ => {
            tracing::info!("event ignored: event_id={}, event={:?}", event_id, event);
        }
    }
}

pub fn record_external_event(status: String) {
    set_status(status);
}

pub async fn handle_interconnect_message(event_payload: String) {
    tracing::info!("handle_interconnect_message payload={}", event_payload);

    let parsed: Value = match serde_json::from_str(&event_payload) {
        Ok(v) => v,
        Err(_) => return,
    };

    let is_ready = check_is_ready(&parsed);
    let count = find_count_value(&parsed);
    let list = find_backup_list(&parsed);
    let is_push_success = check_push_success(&parsed);

    let pending = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_action.clone()
    };

    if is_ready {
        tracing::info!("watch app is ready, pending_action={:?}", pending);
        if let Some(ref action) = pending {
            {
                let mut state = ui_state()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.pending_action = None;
            }
            if action == "backup" {
                send_backup_request_message().await;
                return;
            } else if action == "push" {
                send_push_payload_message().await;
                return;
            }
        }
    }

    if let Some(list) = list {
        let count = list.len();
        let text = format_totp_list_to_text(&list);
        set_input_text_and_status(text, format!("备份成功：已从手环导入 {count} 个账号"));
        clear_pending_action();
    } else if is_push_success {
        set_busy("推送成功：手环端已接收并导入账号".to_string(), false);
        clear_pending_action();
    } else if let Some(count) = count {
        if pending.is_none() {
            set_busy(format!("手环端账号数量：{count}"), false);
        }
    } else {
        tracing::warn!("interconnect message did not contain expected payload");
    }
}

fn check_is_ready(value: &Value) -> bool {
    if let Value::Object(map) = value {
        if let Some(Value::String(action)) = map.get("action") {
            if action == "ready" {
                return true;
            }
        }
        if let Some(Value::String(data_str)) = map.get("data") {
            if let Ok(inner) = serde_json::from_str::<Value>(data_str) {
                return check_is_ready(&inner);
            }
        }
        for val in map.values() {
            if check_is_ready(val) {
                return true;
            }
        }
    }
    false
}

fn check_push_success(value: &Value) -> bool {
    if let Value::Object(map) = value {
        if let Some(Value::String(action)) = map.get("action") {
            if action == "push_response" {
                if let Some(Value::String(status)) = map.get("status") {
                    return status == "success";
                }
            }
        }
        if let Some(Value::String(data_str)) = map.get("data") {
            if let Ok(inner) = serde_json::from_str::<Value>(data_str) {
                return check_push_success(&inner);
            }
        }
        for val in map.values() {
            if check_push_success(val) {
                return true;
            }
        }
    }
    false
}

fn clear_pending_action() {
    let mut state = ui_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.pending_action = None;
    state.pending_payload = None;
}

async fn send_backup_request_message() {
    tracing::info!("send_backup_request_message start");
    let devices = device::get_connected_device_list().await;
    let Some(target_device) = devices.into_iter().next() else {
        return;
    };
    let request = serde_json::json!({
        "action": "backup_request"
    })
    .to_string();
    let _ = interconnect::send_qaic_message(&target_device.addr, PACKAGE_NAME, &request).await;
}

async fn send_push_payload_message() {
    tracing::info!("send_push_payload_message start");
    let devices = device::get_connected_device_list().await;
    let Some(target_device) = devices.into_iter().next() else {
        return;
    };
    let payload = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_payload.clone()
    };
    if let Some(payload) = payload {
        let _ = interconnect::send_qaic_message(&target_device.addr, PACKAGE_NAME, &payload).await;
    }
}

async fn query_wear_account_count_with_msg(status_msg: &str) {
    tracing::info!("query_wear_account_count start");
    set_busy(status_msg.to_string(), true);

    let devices = device::get_connected_device_list().await;
    tracing::info!("query connected device count={}", devices.len());
    let Some(target_device) = devices.into_iter().next() else {
        set_busy("未找到已连接设备".to_string(), false);
        return;
    };

    let apps = match thirdpartyapp::get_thirdparty_app_list(&target_device.addr).await {
        Ok(apps) => apps,
        Err(_) => {
            tracing::error!("query get_thirdparty_app_list failed");
            set_busy("获取快应用列表失败".to_string(), false);
            return;
        }
    };

    let Some(app) = apps
        .iter()
        .find(|app| app.package_name == PACKAGE_NAME)
        .cloned()
    else {
        set_busy("未在手环上找到验证器快应用".to_string(), false);
        return;
    };

    if let Err(_) = register::register_interconnect_recv(&target_device.addr, PACKAGE_NAME).await {
        set_busy("监听接收失败：请确保在 AstroBox 中允许了该插件的 interconnect 权限".to_string(), false);
        return;
    }

    {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_action = Some("query".to_string());
    }

    let _ = thirdpartyapp::launch_qa(&target_device.addr, &app, "pages/index").await;
    let request = serde_json::json!({
        "action": "query_status",
        "requestId": "account_count"
    })
    .to_string();

    match interconnect::send_qaic_message(&target_device.addr, PACKAGE_NAME, &request).await {
        Ok(()) => set_busy("已发送查询请求，等待手环端回传账号数量".to_string(), false),
        Err(_) => set_busy("发送查询请求失败".to_string(), false),
    }
}

async fn query_wear_account_count() {
    query_wear_account_count_with_msg("正在查询手环端账号数量...").await;
}

async fn backup_wear_keys() {
    tracing::info!("backup_wear_keys start");
    set_busy("正在连接设备并备份密钥...".to_string(), true);

    let devices = device::get_connected_device_list().await;
    let Some(target_device) = devices.into_iter().next() else {
        set_busy("未找到已连接设备".to_string(), false);
        return;
    };

    let apps = match thirdpartyapp::get_thirdparty_app_list(&target_device.addr).await {
        Ok(apps) => apps,
        Err(_) => {
            tracing::error!("get_thirdparty_app_list failed");
            set_busy("获取快应用列表失败".to_string(), false);
            return;
        }
    };

    let Some(app) = apps
        .iter()
        .find(|app| app.package_name == PACKAGE_NAME)
        .cloned()
    else {
        set_busy("未在手环上找到验证器快应用".to_string(), false);
        return;
    };

    if let Err(_) = register::register_interconnect_recv(&target_device.addr, PACKAGE_NAME).await {
        set_busy("监听接收失败：请确保在 AstroBox 中允许了该插件的 interconnect 权限".to_string(), false);
        return;
    }

    {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_action = Some("backup".to_string());
    }

    let _ = thirdpartyapp::launch_qa(&target_device.addr, &app, "pages/index").await;
    let request = serde_json::json!({
        "action": "backup_request"
    })
    .to_string();

    match interconnect::send_qaic_message(&target_device.addr, PACKAGE_NAME, &request).await {
        Ok(()) => set_busy("已发送备份请求，等待手环端回传数据...".to_string(), false),
        Err(_) => set_busy("发送备份请求失败".to_string(), false),
    }
}

async fn edit_input() {
    tracing::info!("edit_input start");
    let current = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.input_text.clone()
    };

    let dialog_input = dialog::show_dialog(
        dialog::DialogType::Input,
        dialog::DialogStyle::System,
        &dialog::DialogInfo {
            title: "输入验证器链接".to_string(),
            content: if current.trim().is_empty() {
                "粘贴 otpauth://totp/... 或 Google Authenticator 导出链接".to_string()
            } else {
                current
            },
            buttons: vec![
                dialog::DialogButton {
                    id: "cancel".to_string(),
                    primary: false,
                    content: "取消".to_string(),
                },
                dialog::DialogButton {
                    id: "save".to_string(),
                    primary: true,
                    content: "保存".to_string(),
                },
            ],
        },
    )
    .await;

    tracing::info!(
        "edit dialog returned: button={}, chars={}",
        dialog_input.clicked_btn_id,
        dialog_input.input_result.chars().count()
    );
    if dialog_input.clicked_btn_id == "save" {
        set_input_text_and_status(
            dialog_input.input_result,
            "已保存输入内容，确认后点击推送输入内容".to_string(),
        );
    } else {
        set_status("已取消输入".to_string());
    }
}

async fn input_and_push() {
    tracing::info!("input_and_push start");
    let mut input = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.input_text.clone()
    };

    tracing::info!(
        "input_and_push stored input chars={}",
        input.chars().count()
    );
    if input.trim().is_empty() {
        tracing::info!("stored input empty; showing fallback input dialog");
        let dialog_input = dialog::show_dialog(
            dialog::DialogType::Input,
            dialog::DialogStyle::System,
            &dialog::DialogInfo {
                title: "验证推送器".to_string(),
                content: "粘贴 otpauth://totp/... 或 otpauth-migration://offline?...".to_string(),
                buttons: vec![
                    dialog::DialogButton {
                        id: "cancel".to_string(),
                        primary: false,
                        content: "取消".to_string(),
                    },
                    dialog::DialogButton {
                        id: "send".to_string(),
                        primary: true,
                        content: "发送".to_string(),
                    },
                ],
            },
        )
        .await;

        tracing::info!(
            "fallback dialog returned: button={}, chars={}",
            dialog_input.clicked_btn_id,
            dialog_input.input_result.chars().count()
        );
        if dialog_input.clicked_btn_id != "send" {
            set_busy("已取消".to_string(), false);
            return;
        }
        input = dialog_input.input_result;
    }

    push_input(&input).await;
}

async fn clipboard_to_input() {
    tracing::info!("clipboard_to_input start");
    set_busy("正在读取剪贴板...".to_string(), true);

    let text = match clipboard::read_text().await {
        Ok(text) => {
            tracing::info!("clipboard read success: chars={}", text.chars().count());
            text
        }
        Err(_) => {
            tracing::error!("clipboard read failed");
            set_busy("读取剪贴板失败".to_string(), false);
            return;
        }
    };

    set_input_text_and_status(
        text,
        "已从剪贴板填入输入框，确认后点击推送输入内容".to_string(),
    );
}

async fn qr_image_to_input() {
    tracing::info!("qr_image_to_input start");
    set_busy("请选择二维码图片...".to_string(), true);

    let file = dialog::pick_file(
        &dialog::PickConfig {
            read: true,
            copy_to: None,
        },
        &dialog::FilterConfig {
            multiple: false,
            extensions: vec!["png".to_string(), "jpg".to_string(), "jpeg".to_string()],
            default_directory: "".to_string(),
            default_file_name: "".to_string(),
        },
    )
    .await;

    tracing::info!("file picked: name={}, bytes={}", file.name, file.data.len());
    let qr_text = match decode_qr_image(&file.data) {
        Ok(text) => {
            tracing::info!("qr decode success: chars={}", text.chars().count());
            text
        }
        Err(error) => {
            tracing::error!("qr decode failed: {}", error);
            set_busy(format!("二维码解析失败: {error}"), false);
            return;
        }
    };

    set_input_text_and_status(
        qr_text,
        "已从二维码填入输入框，确认后点击推送输入内容".to_string(),
    );
}

async fn push_input(input: &str) {
    tracing::info!("push_input start: chars={}", input.chars().count());
    let totp_list = match parse_totp_input(input) {
        ParseResult::Ok(list) => {
            tracing::info!("parse success: count={}", list.len());
            list
        }
        ParseResult::Err(error) => {
            tracing::error!("parse failed: {}", error);
            set_busy(format!("解析失败: {error}"), false);
            return;
        }
    };

    set_busy("正在查找已连接设备...".to_string(), true);
    let devices = device::get_connected_device_list().await;
    tracing::info!("connected device count={}", devices.len());
    let Some(target_device) = devices.into_iter().next() else {
        set_busy("未找到已连接设备".to_string(), false);
        return;
    };
    tracing::info!(
        "target device selected: name={}, addr={}",
        target_device.name,
        target_device.addr
    );

    set_busy(
        format!("正在检查 {} 上的快应用...", target_device.name),
        true,
    );
    let apps = match thirdpartyapp::get_thirdparty_app_list(&target_device.addr).await {
        Ok(apps) => {
            tracing::info!("thirdparty app count={}", apps.len());
            apps
        }
        Err(_) => {
            tracing::error!("get_thirdparty_app_list failed");
            set_busy("获取快应用列表失败".to_string(), false);
            return;
        }
    };

    let Some(app) = apps
        .iter()
        .find(|app| app.package_name == PACKAGE_NAME)
        .cloned()
    else {
        tracing::warn!("wear app not found: package={}", PACKAGE_NAME);
        set_busy("未在手环上找到验证器快应用".to_string(), false);
        return;
    };
    tracing::info!("wear app found: package={}", app.package_name);

    if let Err(_) = register::register_interconnect_recv(&target_device.addr, PACKAGE_NAME).await {
        set_busy("监听接收失败：请确保在 AstroBox 中允许了该插件的 interconnect 权限".to_string(), false);
        return;
    }

    set_busy("正在启动快应用...".to_string(), true);
    let _ = thirdpartyapp::launch_qa(&target_device.addr, &app, "pages/index").await;
    tracing::info!("launch_qa requested");

    let count = totp_list.len();
    let payload = match serde_json::to_string(&PushPayload { list: totp_list }) {
        Ok(payload) => {
            tracing::info!("payload serialized: bytes={}", payload.len());
            payload
        }
        Err(error) => {
            tracing::error!("payload serialize failed: {}", error);
            set_busy(format!("生成数据失败: {error}"), false);
            return;
        }
    };

    {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_action = Some("push".to_string());
        state.pending_payload = Some(payload.clone());
    }

    set_busy("正在推送数据...".to_string(), true);
    match interconnect::send_qaic_message(&target_device.addr, PACKAGE_NAME, &payload).await {
        Ok(()) => {
            tracing::info!("send_qaic_message success: count={}", count);
            set_busy("已推送数据，等待手环确认...".to_string(), true);
        }
        Err(_) => {
            tracing::error!("send_qaic_message failed");
            set_busy("发送失败".to_string(), false)
        }
    }
}

pub fn render_main_ui(element_id: &str) {
    tracing::info!("render_main_ui start: element_id={}", element_id);
    let should_check_install = {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.root_element_id = Some(element_id.to_string());
        if state.status.is_empty() {
            state.status = "正在检测手环端应用...".to_string();
        }
        if state.install_check_started {
            false
        } else {
            state.install_check_started = true;
            true
        }
    };

    ui::render(element_id, build_main_ui());
    tracing::info!("render_main_ui rendered: element_id={}", element_id);

    if should_check_install {
        astrobox_ng_wit::block_on(async {
            check_wear_app_installation().await;
        });
    }
}

async fn check_wear_app_installation() {
    tracing::info!("check_wear_app_installation start");
    let devices = device::get_connected_device_list().await;
    tracing::info!("install check connected device count={}", devices.len());
    let Some(target_device) = devices.into_iter().next() else {
        set_status("未连接手环，无法检测手环端应用".to_string());
        return;
    };

    let apps = match thirdpartyapp::get_thirdparty_app_list(&target_device.addr).await {
        Ok(apps) => {
            tracing::info!("install check thirdparty app count={}", apps.len());
            apps
        }
        Err(_) => {
            tracing::error!("install check get_thirdparty_app_list failed");
            set_status(format!("{}：获取手环端应用列表失败", target_device.name));
            return;
        }
    };

    let installed = apps.iter().any(|app| app.package_name == PACKAGE_NAME);
    if installed {
        query_wear_account_count_with_msg(&format!("{}：手环端应用已安装，正在自动查询手环账号数量...", target_device.name)).await;
    } else {
        set_status(format!("{}：手环端应用未安装", target_device.name));
    }
}

fn set_status(status: String) {
    tracing::info!("set_status: {}", status);
    {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.status = status;
    }

    rerender();
}

fn set_busy(status: String, busy: bool) {
    tracing::info!("set_busy: busy={}, status={}", busy, status);
    let root_element_id = {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.status = status;
        state.busy = busy;
        state.root_element_id.clone()
    };

    if let Some(root_element_id) = root_element_id {
        ui::render(&root_element_id, build_main_ui());
    }
}

fn is_busy() -> bool {
    let state = ui_state()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.busy
}

fn rerender() {
    let root_element_id = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.root_element_id.clone()
    };

    if let Some(root_element_id) = root_element_id {
        ui::render(&root_element_id, build_main_ui());
    }
}

fn remember_input_text(input_text: String) {
    {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.input_text = input_text;
    }
}

fn set_input_text_and_status(input_text: String, status: String) {
    let root_element_id = {
        let mut state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.input_text = input_text;
        state.status = status;
        state.busy = false;
        state.root_element_id.clone()
    };

    if let Some(root_element_id) = root_element_id {
        ui::render(&root_element_id, build_main_ui());
    }
}

fn build_main_ui() -> ui::Element {
    let (input_text, input_chars, status, busy) = {
        let state = ui_state()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            state.input_text.clone(),
            state.input_text.chars().count(),
            state.status.clone(),
            state.busy,
        )
    };

    let input_content = (!input_text.trim().is_empty()).then_some(input_text.as_str());
    let input = ui::Element::new(ui::ElementType::Textarea, input_content)
        .width_full()
        .height(156)
        .padding(14)
        .radius(8)
        .border(1, "#6B7280")
        .bg("#F9FAFB")
        .size(16)
        .text_color("#000000")
        .autofocus()
        .tab_index(0)
        .on(ui::Event::Input, INPUT_EVENT)
        .on(ui::Event::Change, INPUT_EVENT);

    let help = ui::Element::new(
        ui::ElementType::P,
        Some("支持普通 TOTP URI、Google Authenticator 导出 URI。二维码图片需从“文件”中选择。"),
    )
    .size(14)
    .text_color("#4B5563")
    .margin_top(14);

    let mut send_button = ui::Element::new(
        ui::ElementType::Button,
        Some(if busy {
            "处理中..."
        } else {
            "推送输入内容"
        }),
    )
    .width_full()
    .height(56)
    .padding(16)
    .margin_top(14)
    .radius(8)
    .bg("#2563EB")
    .text_color("#FFFFFF")
    .on(ui::Event::Click, SEND_EVENT)
    .on(ui::Event::PointerUp, SEND_EVENT);

    if busy {
        send_button = send_button.disabled().opacity(0.72);
    }

    let mut edit_button = ui::Element::new(
        ui::ElementType::Button,
        Some(if busy {
            "处理中..."
        } else {
            "手动输入/编辑"
        }),
    )
    .width_full()
    .height(56)
    .padding(16)
    .margin_top(10)
    .radius(8)
    .bg("#374151")
    .text_color("#FFFFFF")
    .on(ui::Event::Click, EDIT_EVENT)
    .on(ui::Event::PointerUp, EDIT_EVENT);

    if busy {
        edit_button = edit_button.disabled().opacity(0.72);
    }

    let mut clipboard_button = ui::Element::new(
        ui::ElementType::Button,
        Some(if busy {
            "处理中..."
        } else {
            "读取剪贴板"
        }),
    )
    .width_full()
    .height(56)
    .padding(16)
    .margin_top(10)
    .radius(8)
    .bg("#059669")
    .text_color("#FFFFFF")
    .on(ui::Event::Click, CLIPBOARD_EVENT)
    .on(ui::Event::PointerUp, CLIPBOARD_EVENT);

    if busy {
        clipboard_button = clipboard_button.disabled().opacity(0.72);
    }

    let mut qr_image_button = ui::Element::new(
        ui::ElementType::Button,
        Some(if busy {
            "处理中..."
        } else {
            "选择二维码图片文件"
        }),
    )
    .width_full()
    .height(56)
    .padding(16)
    .margin_top(10)
    .radius(8)
    .bg("#7C3AED")
    .text_color("#FFFFFF")
    .on(ui::Event::Click, QR_IMAGE_EVENT)
    .on(ui::Event::PointerUp, QR_IMAGE_EVENT);

    if busy {
        qr_image_button = qr_image_button.disabled().opacity(0.72);
    }

    let mut backup_keys_button = ui::Element::new(
        ui::ElementType::Button,
        Some(if busy {
            "处理中..."
        } else {
            "直接从手环备份密钥"
        }),
    )
    .width_full()
    .height(56)
    .padding(16)
    .margin_top(10)
    .radius(8)
    .bg("#D97706")
    .text_color("#FFFFFF")
    .on(ui::Event::Click, BACKUP_WEAR_KEYS_EVENT)
    .on(ui::Event::PointerUp, BACKUP_WEAR_KEYS_EVENT);

    if busy {
        backup_keys_button = backup_keys_button.disabled().opacity(0.72);
    }

    let loaded_text = if input_chars > 0 {
        format!("当前已载入 {input_chars} 个字符。")
    } else {
        "当前未载入文本。".to_string()
    };

    let loaded = ui::Element::new(ui::ElementType::P, Some(loaded_text.as_str()))
        .size(14)
        .text_color("#374151")
        .margin_top(10);

    let status_text = if status.is_empty() {
        "请先在 AstroBox 中连接手环，并确保已安装 TOTP 快应用".to_string()
    } else {
        status
    };

    let status = ui::Element::new(ui::ElementType::P, Some(status_text.as_str()))
        .size(13)
        .text_color("#374151")
        .margin_top(12);

    ui::Element::new(ui::ElementType::Div, None)
        .flex()
        .flex_direction(ui::FlexDirection::Column)
        .width_full()
        .padding(16)
        .child(input)
        .child(send_button)
        .child(edit_button)
        .child(clipboard_button)
        .child(qr_image_button)
        .child(backup_keys_button)
        .child(help)
        .child(loaded)
        .child(status)
}

fn extract_event_value(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };

    match value {
        Value::String(text) => text,
        Value::Object(map) => ["value", "text", "input", "content"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .unwrap_or(trimmed)
            .to_string(),
        _ => trimmed.to_string(),
    }
}

fn find_backup_list(value: &Value) -> Option<Vec<TOTPInfo>> {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(action)) = map.get("action") {
                if action == "backup_response" {
                    if let Some(Value::Array(list_val)) = map.get("list") {
                        let mut totp_list = Vec::new();
                        for item in list_val {
                            if let Some(info) = parse_backup_item(item) {
                                totp_list.push(info);
                            }
                        }
                        return Some(totp_list);
                    }
                }
            }

            if let Some(Value::String(data_str)) = map.get("data") {
                if let Ok(inner) = serde_json::from_str::<Value>(data_str) {
                    if let Some(list) = find_backup_list(&inner) {
                        return Some(list);
                    }
                }
            }

            for val in map.values() {
                if let Some(list) = find_backup_list(val) {
                    return Some(list);
                }
            }
            None
        }
        _ => None,
    }
}

fn parse_backup_item(item: &Value) -> Option<TOTPInfo> {
    let obj = item.as_object()?;
    let key = obj.get("key")?.as_str()?.to_string();
    let name = obj.get("name")?.as_str()?.to_string();
    let usr = obj.get("usr").and_then(Value::as_str).unwrap_or("").to_string();

    Some(TOTPInfo {
        name,
        usr,
        key,
        algorithm: "SHA1".to_string(),
        digits: 6,
        period: 30,
    })
}

fn percent_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            byte => {
                output.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    output
}

fn format_totp_list_to_text(list: &[TOTPInfo]) -> String {
    let mut lines = Vec::new();
    for item in list {
        let label = if item.usr.is_empty() {
            percent_encode(&item.name)
        } else {
            percent_encode(&format!("{}:{}", item.name, item.usr))
        };
        let secret = item.key.to_uppercase();
        let issuer = percent_encode(&item.name);
        let uri = format!("otpauth://totp/{}?secret={}&issuer={}", label, secret, issuer);
        lines.push(uri);
    }
    lines.join("\n")
}

fn find_count_value(value: &Value) -> Option<u64> {
    match value {
        Value::Object(map) => {
            for key in ["count", "accountCount", "totpCount"] {
                if let Some(count) = map.get(key).and_then(Value::as_u64) {
                    return Some(count);
                }
            }

            if let Some(Value::String(data)) = map.get("data") {
                if let Ok(inner) = serde_json::from_str::<Value>(data) {
                    if let Some(count) = find_count_value(&inner) {
                        return Some(count);
                    }
                }
            }

            for value in map.values() {
                if let Some(count) = find_count_value(value) {
                    return Some(count);
                }
            }
            None
        }
        Value::Array(items) => items.iter().find_map(find_count_value),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .ok()
            .and_then(|inner| find_count_value(&inner)),
        _ => None,
    }
}

fn decode_qr_image(data: &[u8]) -> Result<String, String> {
    let image = image::load_from_memory(data)
        .map_err(|error| format!("图片读取失败: {error}"))?
        .to_luma8();

    let mut attempts: Vec<(&str, GrayImage)> = vec![("original", image.clone())];
    for level in [128, 160, 180, 200] {
        let thresholded = threshold_image(&image, level);
        attempts.push(("threshold", thresholded.clone()));
        attempts.push(("threshold-crop", crop_and_pad_image(&thresholded, 24)));
    }

    for (name, attempt) in attempts {
        match decode_qr_from_luma(&attempt) {
            Ok(content) => {
                tracing::info!(
                    "qr decode succeeded with attempt={}: chars={}",
                    name,
                    content.chars().count()
                );
                return Ok(content);
            }
            Err(error) => {
                tracing::warn!(
                    "qr decode attempt failed: attempt={}, error={}",
                    name,
                    error
                );
            }
        }
    }

    Err("未识别到二维码内容".to_string())
}

fn decode_qr_from_luma(image: &GrayImage) -> Result<String, String> {
    let mut prepared = rqrr::PreparedImage::prepare(image.clone());
    let grids = prepared.detect_grids();
    tracing::info!("qr grids detected: count={}", grids.len());
    let mut contents = Vec::new();

    for grid in grids {
        match grid.decode() {
            Ok((_meta, content)) => {
                let content = content.trim().to_string();
                tracing::info!("qr content decoded: chars={}", content.chars().count());
                if !content.is_empty() {
                    contents.push(content);
                }
            }
            Err(error) => {
                tracing::warn!("qr grid decode failed: {:?}", error);
            }
        }
    }

    if contents.is_empty() {
        Err("未识别到二维码内容".to_string())
    } else {
        Ok(contents.join("\n"))
    }
}

fn threshold_image(image: &GrayImage, level: u8) -> GrayImage {
    let mut out = GrayImage::new(image.width(), image.height());
    for (x, y, pixel) in image.enumerate_pixels() {
        let value = if pixel.0[0] < level { 0 } else { 255 };
        out.put_pixel(x, y, Luma([value]));
    }
    out
}

fn crop_and_pad_image(image: &GrayImage, quiet: u32) -> GrayImage {
    let (mut min_x, mut min_y) = (image.width(), image.height());
    let (mut max_x, mut max_y) = (0, 0);

    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[0] < 128 {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }

    if min_x > max_x || min_y > max_y {
        return image.clone();
    }

    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    let cropped = imageops::crop_imm(image, min_x, min_y, width, height).to_image();
    let mut out = GrayImage::from_pixel(width + quiet * 2, height + quiet * 2, Luma([255]));
    imageops::replace(&mut out, &cropped, quiet as i64, quiet as i64);
    out
}

fn parse_totp_input(input: &str) -> ParseResult {
    let trimmed = input.trim();
    if (trimmed.starts_with('[') && trimmed.ends_with(']')) || (trimmed.starts_with('{') && trimmed.ends_with('}')) {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let mut list = Vec::new();
            let items = match value {
                Value::Array(arr) => arr,
                Value::Object(_) => vec![value],
                _ => Vec::new(),
            };
            for item in items {
                if let Value::Object(obj) = item {
                    let name = obj.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let usr = obj.get("usr")
                        .map(|v| {
                            if v.is_null() {
                                "".to_string()
                            } else if let Some(s) = v.as_str() {
                                s.to_string()
                            } else if let Some(n) = v.as_i64() {
                                n.to_string()
                            } else if let Some(b) = v.as_bool() {
                                b.to_string()
                            } else {
                                "".to_string()
                            }
                        })
                        .unwrap_or_else(|| "".to_string());
                    let key = obj.get("key").and_then(Value::as_str).unwrap_or("").to_string();
                    if !key.is_empty() {
                        list.push(TOTPInfo {
                            name,
                            usr,
                            key,
                            algorithm: "SHA1".to_string(),
                            digits: 6,
                            period: 30,
                        });
                    }
                }
            }
            if !list.is_empty() {
                tracing::info!("parse_totp_input success (JSON format): count={}", list.len());
                return ParseResult::Ok(list);
            }
        }
    }

    let parts = split_input_entries(input);
    if parts.is_empty() {
        return ParseResult::Err("URI 为空".to_string());
    }

    let mut all = Vec::new();
    let mut errors = Vec::new();

    for part in parts {
        let result = if strip_case_insensitive_prefix(part, "otpauth-migration://offline").is_some()
        {
            parse_google_migration_uri(part)
        } else {
            match parse_totp_uri(part) {
                Ok(totp) => ParseResult::Ok(vec![totp]),
                Err(error) => ParseResult::Err(error),
            }
        };

        match result {
            ParseResult::Ok(mut list) => all.append(&mut list),
            ParseResult::Err(error) => errors.push(error),
        }
    }

    if !all.is_empty() {
        tracing::info!(
            "parse_totp_input success: entries={}, count={}",
            all.len() + errors.len(),
            all.len()
        );
        ParseResult::Ok(all)
    } else {
        ParseResult::Err(
            errors
                .into_iter()
                .next()
                .unwrap_or_else(|| "没有解析到可用账号".to_string()),
        )
    }
}

fn split_input_entries(input: &str) -> Vec<&str> {
    input
        .lines()
        .flat_map(|line| line.split('\u{0}'))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn parse_totp_uri(uri: &str) -> Result<TOTPInfo, String> {
    let Some(rest) = strip_case_insensitive_prefix(uri, "otpauth://totp/") else {
        return Err("URI 格式错误或不是 TOTP 类型".to_string());
    };

    let (label_raw, query) = match rest.split_once('?') {
        Some((label, query)) => (label, query),
        None => (rest, ""),
    };

    let label = percent_decode(label_raw);
    let (issuer_from_path, account) = match label.split_once(':') {
        Some((issuer, account)) if !issuer.trim().is_empty() => {
            (issuer.trim().to_string(), account.trim().to_string())
        }
        _ => ("".to_string(), label.trim().to_string()),
    };

    let secret = match get_query_param(query, "secret") {
        Some(secret) if !secret.is_empty() => secret,
        _ => return Err("缺少必需参数: secret".to_string()),
    };

    let issuer = get_query_param(query, "issuer")
        .filter(|issuer| !issuer.is_empty())
        .unwrap_or_else(|| {
            if issuer_from_path.is_empty() {
                "Unknown".to_string()
            } else {
                issuer_from_path
            }
        });

    let algorithm = get_query_param(query, "algorithm")
        .unwrap_or_else(|| "SHA1".to_string())
        .to_uppercase();

    let digits = get_query_param(query, "digits")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(6);

    let period = get_query_param(query, "period")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(30);

    Ok(TOTPInfo {
        name: issuer,
        usr: if account.is_empty() {
            "Unknown".to_string()
        } else {
            account
        },
        key: secret,
        algorithm,
        digits,
        period,
    })
}

fn parse_google_migration_uri(uri: &str) -> ParseResult {
    let query = match uri.split_once('?') {
        Some((_, query)) => query,
        None => return ParseResult::Err("Google 导出 URI 缺少 data 参数".to_string()),
    };

    let Some(encoded_payload) = get_query_param_preserve_plus(query, "data") else {
        return ParseResult::Err("Google 导出 URI 缺少 data 参数".to_string());
    };

    let payload = match decode_google_payload(&encoded_payload) {
        Ok(payload) => payload,
        Err(error) => return ParseResult::Err(error),
    };

    match parse_migration_payload(&payload) {
        Ok(list) if list.is_empty() => {
            ParseResult::Err("Google 导出数据中没有 TOTP 账号".to_string())
        }
        Ok(list) => ParseResult::Ok(list),
        Err(error) => ParseResult::Err(error),
    }
}

fn decode_google_payload(data: &str) -> Result<Vec<u8>, String> {
    let normalized = data
        .trim()
        .replace(' ', "+")
        .replace(['\r', '\n', '\t'], "");
    tracing::info!(
        "decode_google_payload start: chars={}, normalized_chars={}",
        data.chars().count(),
        normalized.chars().count()
    );

    let mut candidates = vec![normalized.clone()];
    let url_safe = normalized.replace('+', "-").replace('/', "_");
    if url_safe != normalized {
        candidates.push(url_safe);
    }
    let standard = normalized.replace('-', "+").replace('_', "/");
    if standard != normalized {
        candidates.push(standard);
    }

    for candidate in candidates.into_iter() {
        let mut padded_candidates = vec![candidate.clone()];
        let padding = (4 - candidate.len() % 4) % 4;
        if padding > 0 {
            padded_candidates.push(format!("{candidate}{}", "=".repeat(padding)));
        }

        for candidate in padded_candidates {
            if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(&candidate) {
                tracing::info!(
                    "google payload decoded with URL_SAFE_NO_PAD: bytes={}",
                    bytes.len()
                );
                return Ok(bytes);
            }
            if let Ok(bytes) = base64::engine::general_purpose::URL_SAFE.decode(&candidate) {
                tracing::info!(
                    "google payload decoded with URL_SAFE: bytes={}",
                    bytes.len()
                );
                return Ok(bytes);
            }
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&candidate) {
                tracing::info!(
                    "google payload decoded with STANDARD: bytes={}",
                    bytes.len()
                );
                return Ok(bytes);
            }
        }
    }

    Err("Google 导出 data 不是有效的 Base64".to_string())
}

fn get_query_param_preserve_plus(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then(|| percent_decode_with_plus_mode(value, false))
    })
}

fn percent_decode(value: &str) -> String {
    percent_decode_with_plus_mode(value, true)
}

fn percent_decode_with_plus_mode(value: &str, plus_as_space: bool) -> String {
    let mut output = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hi = from_hex(bytes[index + 1]);
                let lo = from_hex(bytes[index + 2]);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    output.push((hi << 4) | lo);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            b'+' if plus_as_space => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8(output)
        .unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into())
}
fn parse_migration_payload(bytes: &[u8]) -> Result<Vec<TOTPInfo>, String> {
    let mut cursor = ProtoCursor::new(bytes);
    let mut list = Vec::new();

    while !cursor.is_done() {
        let (field, wire_type) = cursor.read_key()?;
        match (field, wire_type) {
            (1, 2) => {
                let entry = cursor.read_length_delimited()?;
                if let Some(totp) = parse_migration_otp(entry)? {
                    list.push(totp);
                }
            }
            _ => cursor.skip(wire_type)?,
        }
    }

    Ok(list)
}

fn parse_migration_otp(bytes: &[u8]) -> Result<Option<TOTPInfo>, String> {
    let mut cursor = ProtoCursor::new(bytes);
    let mut secret = Vec::new();
    let mut name = String::new();
    let mut issuer = String::new();
    let mut algorithm = 1;
    let mut digits = 1;
    let mut otp_type = 2;

    while !cursor.is_done() {
        let (field, wire_type) = cursor.read_key()?;
        match (field, wire_type) {
            (1, 2) => secret = cursor.read_length_delimited()?.to_vec(),
            (2, 2) => name = decode_proto_string(cursor.read_length_delimited()?)?,
            (3, 2) => issuer = decode_proto_string(cursor.read_length_delimited()?)?,
            (4, 0) => algorithm = cursor.read_varint()? as u32,
            (5, 0) => digits = cursor.read_varint()? as u32,
            (6, 0) => otp_type = cursor.read_varint()? as u32,
            _ => cursor.skip(wire_type)?,
        }
    }

    if otp_type != 0 && otp_type != 2 {
        return Ok(None);
    }
    if secret.is_empty() {
        return Ok(None);
    }

    let (issuer_from_name, account) = match name.split_once(':') {
        Some((issuer, account)) if !issuer.trim().is_empty() => {
            (issuer.trim().to_string(), account.trim().to_string())
        }
        _ => ("".to_string(), name.trim().to_string()),
    };

    let display_issuer = if issuer.trim().is_empty() {
        if issuer_from_name.is_empty() {
            "Unknown".to_string()
        } else {
            issuer_from_name
        }
    } else {
        issuer.trim().to_string()
    };

    Ok(Some(TOTPInfo {
        name: display_issuer,
        usr: if account.is_empty() {
            "Unknown".to_string()
        } else {
            account
        },
        key: base32_encode_no_padding(&secret),
        algorithm: match algorithm {
            2 => "SHA256",
            3 => "SHA512",
            4 => "MD5",
            _ => "SHA1",
        }
        .to_string(),
        digits: match digits {
            2 => 8,
            _ => 6,
        },
        period: 30,
    }))
}

fn decode_proto_string(bytes: &[u8]) -> Result<String, String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| "Google 导出数据包含无效 UTF-8".to_string())
}

fn base32_encode_no_padding(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut output = String::new();
    let mut buffer = 0u16;
    let mut bits = 0u8;

    for byte in bytes {
        buffer = (buffer << 8) | (*byte as u16);
        bits += 8;
        while bits >= 5 {
            let index = ((buffer >> (bits - 5)) & 0b11111) as usize;
            output.push(ALPHABET[index] as char);
            bits -= 5;
        }
    }

    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0b11111) as usize;
        output.push(ALPHABET[index] as char);
    }

    output
}

struct ProtoCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ProtoCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn read_key(&mut self) -> Result<(u32, u8), String> {
        let key = self.read_varint()?;
        Ok(((key >> 3) as u32, (key & 0b111) as u8))
    }

    fn read_varint(&mut self) -> Result<u64, String> {
        let mut result = 0u64;
        let mut shift = 0u32;

        loop {
            if self.pos >= self.bytes.len() {
                return Err("Google 导出 protobuf 数据不完整".to_string());
            }
            let byte = self.bytes[self.pos];
            self.pos += 1;
            result |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
            if shift >= 64 {
                return Err("Google 导出 protobuf varint 无效".to_string());
            }
        }
    }

    fn read_length_delimited(&mut self) -> Result<&'a [u8], String> {
        let len = self.read_varint()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "Google 导出 protobuf 长度溢出".to_string())?;
        if end > self.bytes.len() {
            return Err("Google 导出 protobuf 数据长度不完整".to_string());
        }
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }

    fn skip(&mut self, wire_type: u8) -> Result<(), String> {
        match wire_type {
            0 => {
                self.read_varint()?;
                Ok(())
            }
            1 => self.skip_bytes(8),
            2 => {
                let len = self.read_varint()? as usize;
                self.skip_bytes(len)
            }
            5 => self.skip_bytes(4),
            _ => Err("Google 导出 protobuf 包含不支持的 wire type".to_string()),
        }
    }

    fn skip_bytes(&mut self, len: usize) -> Result<(), String> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "Google 导出 protobuf 长度溢出".to_string())?;
        if end > self.bytes.len() {
            return Err("Google 导出 protobuf 数据长度不完整".to_string());
        }
        self.pos = end;
        Ok(())
    }
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }

    let (candidate, rest) = value.split_at(prefix.len());
    candidate.eq_ignore_ascii_case(prefix).then_some(rest)
}

fn get_query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name == key).then(|| percent_decode(value))
    })
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
