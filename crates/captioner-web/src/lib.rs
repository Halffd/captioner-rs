use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        Html, IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::broadcast;
use tokio_stream::Stream;
use tracing::{error, info};

use captioner_core::AppCommand;

#[derive(Clone)]
struct WebState {
    transcript: Arc<Mutex<Vec<String>>>,
    event_tx: broadcast::Sender<String>,
    command_tx: broadcast::Sender<AppCommand>,
    recording: Arc<Mutex<bool>>,
    current_model: Arc<Mutex<String>>,
    current_language: Arc<Mutex<Option<String>>>,
}

#[derive(Serialize)]
struct TranscriptResponse {
    text: Vec<String>,
}

#[derive(Serialize)]
struct StatusResponse {
    recording: bool,
    model: String,
    language: Option<String>,
    line_count: usize,
}

#[derive(Deserialize)]
struct SetModelRequest {
    model: String,
}

#[derive(Deserialize)]
struct SetLanguageRequest {
    language: Option<String>,
}

#[derive(Deserialize)]
struct SetRecordingScaleRequest {
    scale: f32,
}

async fn get_transcript(State(state): State<WebState>) -> Json<TranscriptResponse> {
    Json(TranscriptResponse {
        text: state.transcript.lock().clone(),
    })
}

async fn get_status(State(state): State<WebState>) -> Json<StatusResponse> {
    Json(StatusResponse {
        recording: *state.recording.lock(),
        model: state.current_model.lock().clone(),
        language: state.current_language.lock().clone(),
        line_count: state.transcript.lock().len(),
    })
}

async fn transcript_events(
    State(state): State<WebState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>> + Send + 'static> {
    let rx = state.event_tx.subscribe();

    struct EventStream {
        rx: broadcast::Receiver<String>,
    }

    impl Stream for EventStream {
        type Item = Result<Event, Infallible>;

        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            loop {
                match self.rx.try_recv() {
                    Ok(msg) => {
                        return std::task::Poll::Ready(Some(Ok(Event::default().data(msg))));
                    }
                    Err(broadcast::error::TryRecvError::Empty) => {
                        cx.waker().wake_by_ref();
                        return std::task::Poll::Pending;
                    }
                    Err(broadcast::error::TryRecvError::Closed) => {
                        return std::task::Poll::Ready(None);
                    }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => {
                        continue;
                    }
                }
            }
        }
    }

    Sse::new(EventStream { rx })
}

async fn clear_transcript(State(state): State<WebState>) -> StatusCode {
    state.transcript.lock().clear();
    let _ = state.event_tx.send("CLEAR".to_string());
    let _ = state.command_tx.send(AppCommand::Clear);
    StatusCode::OK
}

async fn toggle_recording(State(state): State<WebState>) -> Json<StatusResponse> {
    let _ = state.command_tx.send(AppCommand::ToggleRecording);
    let mut rec = state.recording.lock();
    *rec = !*rec;
    let now_recording = *rec;
    drop(rec);
    Json(StatusResponse {
        recording: now_recording,
        model: state.current_model.lock().clone(),
        language: state.current_language.lock().clone(),
        line_count: state.transcript.lock().len(),
    })
}

async fn set_model(
    State(state): State<WebState>,
    Json(body): Json<SetModelRequest>,
) -> StatusCode {
    let _ = state.command_tx.send(AppCommand::SetModel(body.model.clone()));
    *state.current_model.lock() = body.model.clone();
    let _ = state.event_tx.send(format!("MODEL_CHANGED:{}", body.model));
    StatusCode::OK
}

async fn set_language(
    State(state): State<WebState>,
    Json(body): Json<SetLanguageRequest>,
) -> StatusCode {
    let _ = state.command_tx.send(AppCommand::SetLanguage(body.language.clone()));
    *state.current_language.lock() = body.language.clone();
    let lang_str = body.language.as_deref().unwrap_or("");
    let _ = state.event_tx.send(format!("LANGUAGE_CHANGED:{}", lang_str));
    StatusCode::OK
}

async fn set_recording_scale(
    State(state): State<WebState>,
    Json(body): Json<SetRecordingScaleRequest>,
) -> StatusCode {
    let _ = state.command_tx.send(AppCommand::SetRecordingScale(body.scale));
    StatusCode::OK
}

async fn quit(State(state): State<WebState>) -> StatusCode {
    let _ = state.command_tx.send(AppCommand::Quit);
    StatusCode::OK
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: WebState) {
    let mut rx = state.event_tx.subscribe();

    let (mut ws_sender, mut ws_receiver) = socket.split();

    let send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let ws_msg = Message::Text(msg);
                    if ws_sender.send(ws_msg).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    let cmd_tx = state.command_tx.clone();
    let rec = state.recording.clone();
    let model = state.current_model.clone();
    let lang = state.current_language.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_receiver.next().await {
            if let Message::Text(text) = msg {
                let parts: Vec<&str> = text.splitn(2, ':').collect();
                match parts.first().copied().unwrap_or("") {
                    "toggle_recording" => {
                        let _ = cmd_tx.send(AppCommand::ToggleRecording);
                        let mut r = rec.lock();
                        *r = !*r;
                    }
                    "clear" => {
                        let _ = cmd_tx.send(AppCommand::Clear);
                    }
                    "set_model" => {
                        if let Some(m) = parts.get(1) {
                            let _ = cmd_tx.send(AppCommand::SetModel(m.to_string()));
                            *model.lock() = m.to_string();
                        }
                    }
                    "set_language" => {
                        let l = parts.get(1).and_then(|v| {
                            if v.is_empty() { None } else { Some(v.to_string()) }
                        });
                        let _ = cmd_tx.send(AppCommand::SetLanguage(l.clone()));
                        *lang.lock() = l;
                    }
                    "quit" => {
                        let _ = cmd_tx.send(AppCommand::Quit);
                    }
                    "ping" => {}
                    _ => {}
                }
            }
        }
    });

    tokio::select! {
        _ = send_task => {},
        _ = recv_task => {},
    }
}

pub fn create_app(
    transcript: Arc<Mutex<Vec<String>>>,
    event_tx: broadcast::Sender<String>,
    command_tx: broadcast::Sender<AppCommand>,
    recording: Arc<Mutex<bool>>,
    current_model: Arc<Mutex<String>>,
    current_language: Arc<Mutex<Option<String>>>,
) -> Router {
    let state = WebState {
        transcript,
        event_tx,
        command_tx,
        recording,
        current_model,
        current_language,
    };

    Router::new()
        .route("/", get(index_handler))
        .route("/transcript", get(get_transcript))
        .route("/status", get(get_status))
        .route("/events", get(transcript_events))
        .route("/ws", get(ws_handler))
        .route("/clear", post(clear_transcript))
        .route("/toggle-recording", post(toggle_recording))
        .route("/model", post(set_model))
        .route("/language", post(set_language))
        .route("/recording-scale", post(set_recording_scale))
        .route("/quit", post(quit))
        .with_state(state)
}

async fn index_handler() -> Html<&'static str> {
    Html(HTML_TEMPLATE)
}

pub struct WebConfig {
    pub port: u16,
    pub transcript: Arc<Mutex<Vec<String>>>,
    pub event_tx: broadcast::Sender<String>,
    pub command_tx: broadcast::Sender<AppCommand>,
    pub recording: Arc<Mutex<bool>>,
    pub current_model: Arc<Mutex<String>>,
    pub current_language: Arc<Mutex<Option<String>>>,
}

pub async fn start_server(cfg: WebConfig) {
    let app = create_app(
        cfg.transcript,
        cfg.event_tx,
        cfg.command_tx,
        cfg.recording,
        cfg.current_model,
        cfg.current_language,
    );

    let addr = format!("0.0.0.0:{}", cfg.port);
    info!("Web UI listening on http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind web server: {}", e);
            return;
        }
    };

    axum::serve(listener, app).await.ok();
}

const HTML_TEMPLATE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Captioner</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;background:#1a1a2e;color:#fff;min-height:100vh;display:flex;flex-direction:column}
.header{display:flex;justify-content:space-between;align-items:center;padding:12px 20px;background:#16213e;border-bottom:1px solid #0f3460}
.header h1{font-size:20px;font-weight:600}
.controls{display:flex;gap:8px;align-items:center}
.btn{background:#4a4a6a;border:none;color:#fff;padding:8px 16px;border-radius:6px;cursor:pointer;font-size:14px;transition:background .15s}
.btn:hover{background:#5a5a7a}
.btn-rec{min-width:90px}
.btn-rec.on{background:#2d5a2d}
.btn-rec.off{background:#5a2d2d}
.btn-danger{background:#6a2d2d}
.btn-danger:hover{background:#8a3d3d}
.status-bar{display:flex;gap:16px;align-items:center;padding:8px 20px;background:#16213e80;font-size:13px;color:#aaa}
.status-dot{width:8px;height:8px;border-radius:50%;display:inline-block;margin-right:4px}
.status-dot.on{background:#4caf50}
.status-dot.off{background:#f44336}
.toolbar{display:flex;gap:8px;align-items:center;padding:8px 20px;background:#16213e40}
.toolbar label{font-size:13px;color:#aaa;margin-right:4px}
.toolbar select,.toolbar input{background:#2a2a4a;color:#fff;border:1px solid #0f3460;border-radius:4px;padding:4px 8px;font-size:13px}
.toolbar input{width:70px}
#transcript{flex:1;background:rgba(0,0,0,.3);border-radius:0;padding:20px;font-size:24px;line-height:1.6;white-space:pre-wrap;overflow-y:auto}
.conn-status{font-size:12px;padding:2px 8px;border-radius:4px;margin-left:8px}
.conn-status.connected{background:#2d5a2d;color:#8f8}
.conn-status.disconnected{background:#5a2d2d;color:#f88}
</style>
</head>
<body>
<div class="header">
  <h1>Captioner</h1>
  <div class="controls">
    <span id="connStatus" class="conn-status disconnected">disconnected</span>
    <button id="btnRec" class="btn btn-rec on" onclick="toggleRecording()">&#9679; REC</button>
    <button class="btn" onclick="clearTranscript()">Clear</button>
    <button class="btn btn-danger" onclick="quit()">Quit</button>
  </div>
</div>
<div class="status-bar">
  <span><span id="recDot" class="status-dot on"></span><span id="recLabel">Recording</span></span>
  <span>Model: <strong id="modelLabel">base</strong></span>
  <span>Lang: <strong id="langLabel">--</strong></span>
  <span>Lines: <strong id="lineCount">0</strong></span>
</div>
<div class="toolbar">
  <label>Model:</label>
  <select id="modelSelect" onchange="setModel(this.value)">
    <option value="tiny">tiny</option>
    <option value="base" selected>base</option>
    <option value="small">small</option>
    <option value="medium">medium</option>
    <option value="large">large</option>
    <option value="large-v2">large-v2</option>
    <option value="large-v3">large-v3</option>
    <option value="tiny.en">tiny.en</option>
    <option value="base.en">base.en</option>
    <option value="small.en">small.en</option>
    <option value="medium.en">medium.en</option>
  </select>
  <label>Language:</label>
  <input id="langInput" type="text" placeholder="e.g. en, ja" onchange="setLanguage(this.value)">
</div>
<div id="transcript"></div>
<script>
const transcript = document.getElementById('transcript');
const btnRec = document.getElementById('btnRec');
const recDot = document.getElementById('recDot');
const recLabel = document.getElementById('recLabel');
const modelLabel = document.getElementById('modelLabel');
const langLabel = document.getElementById('langLabel');
const lineCount = document.getElementById('lineCount');
const connStatus = document.getElementById('connStatus');
const modelSelect = document.getElementById('modelSelect');
const langInput = document.getElementById('langInput');

let ws = null;
let recording = true;

function updateRecUI() {
  if (recording) {
    btnRec.className = 'btn btn-rec on';
    btnRec.innerHTML = '&#9679; REC';
    recDot.className = 'status-dot on';
    recLabel.textContent = 'Recording';
  } else {
    btnRec.className = 'btn btn-rec off';
    btnRec.innerHTML = '&#9675; PAUSED';
    recDot.className = 'status-dot off';
    recLabel.textContent = 'Paused';
  }
}

function setConnected(v) {
  connStatus.textContent = v ? 'connected' : 'disconnected';
  connStatus.className = 'conn-status ' + (v ? 'connected' : 'disconnected');
}

function connectWS() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  ws = new WebSocket(proto + '//' + location.host + '/ws');
  ws.onopen = () => { setConnected(true); };
  ws.onclose = () => { setConnected(false); setTimeout(connectWS, 2000); };
  ws.onerror = () => { ws.close(); };
  ws.onmessage = (e) => {
    const data = e.data;
    if (data === 'CLEAR') {
      transcript.textContent = '';
    } else if (data.startsWith('MODEL_CHANGED:')) {
      modelLabel.textContent = data.slice(14);
      modelSelect.value = data.slice(14);
    } else if (data.startsWith('LANGUAGE_CHANGED:')) {
      const l = data.slice(17);
      langLabel.textContent = l || '--';
      langInput.value = l;
    } else {
      transcript.textContent += data + '\n';
      transcript.scrollTop = transcript.scrollHeight;
      updateLineCount();
    }
  };
}

function connectSSE() {
  const es = new EventSource('/events');
  es.onmessage = (e) => {
    if (e.data === 'CLEAR') {
      transcript.textContent = '';
    } else {
      transcript.textContent += e.data + '\n';
      transcript.scrollTop = transcript.scrollHeight;
      updateLineCount();
    }
  };
}

function updateLineCount() {
  const text = transcript.textContent.trim();
  lineCount.textContent = text ? text.split('\n').length : '0';
}

async function loadTranscript() {
  try {
    const res = await fetch('/transcript');
    const data = await res.json();
    transcript.textContent = data.text.join('\n');
    updateLineCount();
  } catch(e) {}
}

async function loadStatus() {
  try {
    const res = await fetch('/status');
    const data = await res.json();
    recording = data.recording;
    modelLabel.textContent = data.model;
    modelSelect.value = data.model;
    langLabel.textContent = data.language || '--';
    langInput.value = data.language || '';
    updateRecUI();
    updateLineCount();
  } catch(e) {}
}

function toggleRecording() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send('toggle_recording');
    recording = !recording;
    updateRecUI();
  } else {
    fetch('/toggle-recording', {method:'POST'}).then(r => r.json()).then(d => {
      recording = d.recording;
      updateRecUI();
    });
  }
}

function clearTranscript() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send('clear');
    transcript.textContent = '';
  } else {
    fetch('/clear', {method:'POST'}).then(() => { transcript.textContent = ''; });
  }
}

function setModel(m) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send('set_model:' + m);
    modelLabel.textContent = m;
  } else {
    fetch('/model', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({model:m})});
    modelLabel.textContent = m;
  }
}

function setLanguage(l) {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send('set_language:' + l);
    langLabel.textContent = l || '--';
  } else {
    fetch('/language', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({language:l||null})});
    langLabel.textContent = l || '--';
  }
}

function quit() {
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send('quit');
  } else {
    fetch('/quit', {method:'POST'});
  }
}

loadTranscript();
loadStatus();
connectWS();
</script>
</body>
</html>
"#;
