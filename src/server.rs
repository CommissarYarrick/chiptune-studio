use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Component, Path, PathBuf},
};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const MAX_BODY: usize = 20 * 1024 * 1024;
const INDEX: &str = include_str!("../web/index.html");
const APP: &str = include_str!("../web/app.js");
const CSS: &str = include_str!("../web/style.css");
const RECORDER: &str = include_str!("../web/recorder.js");

fn inside(root: &Path, value: &str) -> Result<PathBuf> {
    if value.trim().is_empty() {
        return Err(anyhow!("Path must be nonempty"));
    }
    if value.contains(':') {
        return Err(anyhow!("Path must stay below the configured root"));
    }
    let normalized = value.replace('\\', "/");
    let input = Path::new(&normalized);
    if input.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(anyhow!("Path must stay below the configured root"));
    }
    let candidate = root.join(input);
    let mut ancestor = candidate.as_path();
    while !ancestor.exists() {
        ancestor = ancestor.parent().ok_or_else(|| anyhow!("Invalid path"))?;
    }
    let resolved = ancestor.canonicalize()?;
    if !resolved.starts_with(root) {
        return Err(anyhow!("Path must stay below the configured root"));
    }
    let tail = candidate.strip_prefix(ancestor)?;
    if tail.as_os_str().is_empty() { Ok(resolved) } else { Ok(resolved.join(tail)) }
}
fn rel(root: &Path, p: &Path) -> Result<String> {
    Ok(p.canonicalize()?
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/"))
}
fn json_response(status: u16, value: Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    Response::from_data(body)
        .with_status_code(StatusCode(status))
        .with_header(Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap())
}
fn error_response(status: u16, e: impl ToString) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, json!({"error":e.to_string()}))
}
fn parse_body(req: &mut tiny_http::Request) -> Result<Value> {
    if req.body_length().unwrap_or(0) > MAX_BODY {
        return Err(anyhow!("Request is too large (20 MiB limit)"));
    }
    let mut bytes = Vec::new();
    req.as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BODY {
        return Err(anyhow!("Request is too large (20 MiB limit)"));
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid JSON"))
}
fn query_path(url: &str) -> String {
    form_urlencoded::parse(url.split_once('?').map(|x| x.1).unwrap_or("").as_bytes())
        .find(|(k, _)| k == "path")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}
fn project_base(root: &Path, p: &Path) -> PathBuf {
    if let Ok(r) = p.strip_prefix(root) {
        let c: Vec<_> = r.components().collect();
        if c.len() >= 3 && c[0].as_os_str() == "projects" {
            return root.join("projects").join(c[1].as_os_str());
        }
    }
    p.parent().unwrap_or(root).to_path_buf()
}
fn recent_projects(root: &Path) -> Value {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(u128, PathBuf)>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(meta) = fs::symlink_metadata(&p) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                walk(root, &p, out);
            } else if p
                .extension()
                .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("json"))
            {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if name == "manifest.json"
                    || name == "timeline.json"
                    || name.ends_with(".transcription.json")
                {
                    continue;
                }
                if let Ok(r) = p.strip_prefix(root) {
                    out.push((
                        meta.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_nanos())
                            .unwrap_or(0),
                        r.to_path_buf(),
                    ));
                }
            }
        }
    }
    let dir = root.join("projects");
    if fs::symlink_metadata(&dir)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(true)
    {
        return json!({"projects":[]});
    }
    let mut files = Vec::new();
    walk(root, &dir, &mut files);
    files.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    files.truncate(300);
    let mut items:Vec<(u128,Value)>=files.into_iter().filter_map(|(mtime,r)|{let abs=root.join(&r);let inspection=crate::engine::inspect(&abs).ok()?;let score=&inspection["score"];Some((mtime,json!({"path":r.to_string_lossy().replace('\\',"/"),"title":score["title"],"profile":score["profile"],"bpm":score["bpm"]})))}).collect();
    items.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    items.truncate(30);
    json!({"projects":items.into_iter().map(|(_,v)|v).collect::<Vec<_>>()})
}
fn static_response(path: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let (body, mime) = match path {
        "/" => (INDEX, "text/html; charset=utf-8"),
        "/app.js" => (APP, "application/javascript; charset=utf-8"),
        "/style.css" => (CSS, "text/css; charset=utf-8"),
        "/recorder.js" => (RECORDER, "application/javascript; charset=utf-8"),
        _ => return error_response(404, "Resource not found"),
    };
    Response::from_string(body)
        .with_header(Header::from_bytes("Content-Type", mime).unwrap())
        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap())
}
pub fn studio(root: &Path, port: u16) -> Result<()> {
    fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let server = Server::http(("127.0.0.1", port)).map_err(|e| anyhow!(e.to_string()))?;
    for mut req in server.incoming_requests() {
        let result = (|| -> Result<Response<std::io::Cursor<Vec<u8>>>> {
            let url = req.url().to_owned();
            let path = url.split('?').next().unwrap_or(&url);
            if req.method() == &Method::Get {
                return Ok(match path {
                    "/" | "/app.js" | "/style.css" | "/recorder.js" => static_response(path),
                    "/api/info" => json_response(200, crate::engine::describe()),
                    "/api/projects" => json_response(200, recent_projects(&root)),
                    "/api/read" => {
                        let p = inside(&root, &query_path(&url))?;
                        let mut v = crate::engine::inspect(&p)?;
                        v["path"] = json!(rel(&root, &p)?);
                        json_response(200, v)
                    }
                    "/audio" => {
                        let p = inside(&root, &query_path(&url))?;
                        let extension = p
                            .extension()
                            .map(|x| x.to_string_lossy().to_ascii_lowercase())
                            .unwrap_or_default();
                        if p.extension()
                            .is_none_or(|_| !matches!(extension.as_str(), "wav" | "ogg"))
                            || !p.is_file()
                        {
                            error_response(404, "Audio not found")
                        } else {
                            Response::from_data(fs::read(p)?)
                                .with_header(
                                    Header::from_bytes(
                                        "Content-Type",
                                        if extension == "ogg" {
                                            "audio/ogg"
                                        } else {
                                            "audio/wav"
                                        },
                                    )
                                    .unwrap(),
                                )
                                .with_header(
                                    Header::from_bytes("X-Content-Type-Options", "nosniff")
                                        .unwrap(),
                                )
                        }
                    }
                    _ => error_response(404, "Resource not found"),
                });
            }
            if req.method() != &Method::Post {
                return Ok(error_response(404, "Resource not found"));
            }
            if let Some(origin) = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Origin"))
                .map(|h| h.value.as_str())
            {
                let allowed = format!("http://127.0.0.1:{port}");
                let local = format!("http://localhost:{port}");
                if origin != allowed && origin != local {
                    return Ok(error_response(403, "Origin is not allowed"));
                }
            }
            let data = parse_body(&mut req)?;
            if !data.is_object() {
                return Err(anyhow!("Request body must be a JSON object"));
            }
            let response = match path {
                "/api/new" => {
                    let id = uuid::Uuid::new_v4().simple().to_string();
                    let p = inside(&root, &format!("projects/{id}/score.json"))?;
                    let mut v = crate::engine::create(
                        &p,
                        data["profile"].as_str().unwrap_or("chip4"),
                        data["title"].as_str().unwrap_or("New composition"),
                        data["template"].as_str().unwrap_or("empty"),
                    )?;
                    let r = rel(&root, &p)?;
                    v["path"] = json!(r.clone());
                    v["project"] = json!(r);
                    json_response(200, v)
                }
                "/api/transcribe" => {
                    let encoded = data["wav_base64"]
                        .as_str()
                        .ok_or_else(|| anyhow!("Invalid wav_base64"))?;
                    let bytes =
                        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                            .map_err(|_| anyhow!("Invalid wav_base64"))?;
                    if bytes.len() > MAX_BODY {
                        return Err(anyhow!("WAV exceeds the 20 MiB limit"));
                    }
                    let reader = hound::WavReader::new(std::io::Cursor::new(bytes.as_slice()))?;
                    let spec = reader.spec();
                    if spec.sample_rate == 0 {
                        return Err(anyhow!("WAV must be mono PCM16"));
                    }
                    let duration = reader.duration() as f64 / spec.sample_rate as f64;
                    if spec.channels != 1
                        || spec.bits_per_sample != 16
                        || spec.sample_format != hound::SampleFormat::Int
                    {
                        return Err(anyhow!("WAV must be mono PCM16"));
                    }
                    if duration > 60. {
                        return Err(anyhow!("Recording exceeds 60 seconds"));
                    }
                    let tid = uuid::Uuid::new_v4().simple().to_string();
                    let take = inside(&root, &format!("takes/{tid}.wav"))?;
                    fs::create_dir_all(take.parent().unwrap())?;
                    fs::write(&take, bytes)?;
                    let pid = uuid::Uuid::new_v4().simple().to_string();
                    let score = inside(&root, &format!("projects/{pid}/score.json"))?;
                    let result = crate::transcribe::import_take(
                        &take,
                        &score,
                        data["bpm"].as_f64().unwrap_or(120.),
                        data["quantize"].as_f64().unwrap_or(0.25),
                        data["profile"].as_str().unwrap_or("chip4"),
                        data["arrange"].as_bool().unwrap_or(true),
                        data["title"].as_str().unwrap_or("Take"),
                    )?;
                    let mut v = crate::engine::inspect(&score)?;
                    let r = rel(&root, &score)?;
                    v["path"] = json!(r.clone());
                    v["project"] = json!(r);
                    v["result"] = result;
                    json_response(200, v)
                }
                "/api/patch" => {
                    let p = inside(&root, data["path"].as_str().unwrap_or(""))?;
                    let id = uuid::Uuid::new_v4().simple().to_string();
                    let out = inside(
                        &root,
                        &format!(
                            "{}/revisions/{id}.json",
                            project_base(&root, &p)
                                .strip_prefix(&root)?
                                .to_string_lossy()
                        ),
                    )?;
                    let mut v = crate::engine::patch(
                        &p,
                        &data["operations"],
                        &out,
                        data["expected_sha256"].as_str().unwrap_or(""),
                    )?;
                    let r = rel(&root, &out)?;
                    v["path"] = json!(r.clone());
                    v["project"] = json!(r);
                    json_response(200, v)
                }
                "/api/render" => {
                    let p = inside(&root, data["path"].as_str().unwrap_or(""))?;
                    let id = uuid::Uuid::new_v4().simple().to_string();
                    let default = project_base(&root, &p).join("renders").join(id);
                    let out = if let Some(s) = data["out"].as_str() {
                        inside(&root, s)?
                    } else {
                        inside(&root, &default.strip_prefix(&root)?.to_string_lossy())?
                    };
                    let mut v = crate::engine::render(
                        &p,
                        &out,
                        32000,
                        data["from_bar"].as_f64().unwrap_or(0.),
                        data["bars"].as_f64(),
                        true,
                        &[],
                    )?;
                    if let Some(ogg) = v["ogg"].as_str() {
                        let ogg_url = format!(
                            "/audio?path={}",
                            form_urlencoded::byte_serialize(rel(&root, Path::new(ogg))?.as_bytes())
                                .collect::<String>()
                        );
                        v["ogg_url"] = json!(ogg_url);
                    }
                    if let Some(mix) = v["mix"].as_str() {
                        let mp = Path::new(mix);
                        v["audio_url"] = json!(format!(
                            "/audio?path={}",
                            form_urlencoded::byte_serialize(rel(&root, mp)?.as_bytes())
                                .collect::<String>()
                        ));
                    }
                    json_response(200, v)
                }
                _ => error_response(404, "Endpoint not found"),
            };
            Ok(response)
        })();
        let response = result.unwrap_or_else(|e| error_response(400, e));
        let _ = req.respond(response);
    }
    Ok(())
}
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn tools_list() -> Value {
    json!([
     {"name":"chip_describe","description":"Return the chiptune engine's profiles, instruments and score format.","inputSchema":schema(json!({}),&[])},
     {"name":"chip_new_score","description":"Create a score below root.","inputSchema":schema(json!({"path":{"type":"string"},"profile":{"type":"string"},"title":{"type":"string"},"template":{"type":"string"}}),&["path","profile","title","template"])},
     {"name":"chip_read_score","description":"Read a score below root.","inputSchema":schema(json!({"path":{"type":"string"}}),&["path"])},
     {"name":"chip_validate_score","description":"Validate a score below root.","inputSchema":schema(json!({"path":{"type":"string"}}),&["path"])},
     {"name":"chip_patch_score","description":"Apply score operations after checking its hash.","inputSchema":schema(json!({"path":{"type":"string"},"operations":{"type":"array","items":{"type":"object"}},"out":{"type":"string"},"expected_sha256":{"type":"string"}}),&["path","operations","out","expected_sha256"])},
     {"name":"chip_render_score","description":"Render a score below root.","inputSchema":schema(json!({"path":{"type":"string"},"out":{"type":"string"},"sample_rate":{"type":"integer","default":32000},"from_bar":{"type":"number","default":0},"bars":{"type":"number"},"stems":{"type":"boolean","default":true},"solo":{"type":"array","items":{"type":"string"}}}),&["path","out"])},
     {"name":"chip_import_hum","description":"Transcribe a WAV below root into a score below root.","inputSchema":schema(json!({"audio_path":{"type":"string"},"out":{"type":"string"},"bpm":{"type":"number","default":120},"quantize":{"type":"number","default":0.25},"profile":{"type":"string","default":"chip4"},"arrange":{"type":"boolean","default":true}}),&["audio_path","out"])},
     {"name":"chip_compare_renders","description":"Compare render manifests below root.","inputSchema":schema(json!({"paths":{"type":"array","items":{"type":"string"}}}),&["paths"])}
    ])
}
fn tool_call(root: &Path, name: &str, a: &Value) -> Result<Value> {
    let path = |key: &str| -> Result<PathBuf> { inside(root, a[key].as_str().unwrap_or("")) };
    Ok(match name {
        "chip_describe" => crate::engine::describe(),
        "chip_new_score" => crate::engine::create(
            &path("path")?,
            a["profile"].as_str().unwrap_or("chip4"),
            a["title"].as_str().unwrap_or(""),
            a["template"].as_str().unwrap_or("empty"),
        )?,
        "chip_read_score" => crate::engine::inspect(&path("path")?)?,
        "chip_validate_score" => crate::score::validate(&crate::score::load(&path("path")?)?)?,
        "chip_patch_score" => crate::engine::patch(
            &path("path")?,
            &a["operations"],
            &path("out")?,
            a["expected_sha256"].as_str().unwrap_or(""),
        )?,
        "chip_render_score" => {
            let solo = a["solo"]
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            crate::engine::render(
                &path("path")?,
                &path("out")?,
                a["sample_rate"].as_u64().unwrap_or(32000) as u32,
                a["from_bar"].as_f64().unwrap_or(0.),
                a["bars"].as_f64(),
                a["stems"].as_bool().unwrap_or(true),
                &solo,
            )?
        }
        "chip_import_hum" => crate::transcribe::import_take(
            &path("audio_path")?,
            &path("out")?,
            a["bpm"].as_f64().unwrap_or(120.),
            a["quantize"].as_f64().unwrap_or(0.25),
            a["profile"].as_str().unwrap_or("chip4"),
            a["arrange"].as_bool().unwrap_or(true),
            "Take",
        )?,
        "chip_compare_renders" => {
            let paths = a["paths"]
                .as_array()
                .ok_or_else(|| anyhow!("paths must be an array"))?
                .iter()
                .map(|x| inside(root, x.as_str().unwrap_or("")))
                .collect::<Result<Vec<_>>>()?;
            crate::engine::compare(&paths)?
        }
        _ => return Err(anyhow!("Unknown tool: {name}")),
    })
}
fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub fn mcp(root: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in BufReader::new(stdin.lock()).lines() {
        let line = line?;
        let parsed: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                writeln!(stdout, "{}", rpc_error(Value::Null, -32700, "Parse error"))?;
                stdout.flush()?;
                continue;
            }
        };
        let id = parsed.get("id").cloned();
        let method = parsed["method"].as_str().unwrap_or("");
        let params = &parsed["params"];
        if method == "notifications/initialized" || method.starts_with("notifications/") {
            continue;
        }
        let response = match method {
            "initialize" => {
                json!({"protocolVersion":params["protocolVersion"].as_str().filter(|v|["2024-11-05","2025-03-26","2025-06-18"].contains(v)).unwrap_or("2025-06-18"),"capabilities":{"tools":{"listChanged":false},"resources":{"subscribe":false,"listChanged":false}},"serverInfo":{"name":"chiptune","version":"0.2.0"}})
            }
            "ping" => json!({}),
            "tools/list" => json!({"tools":tools_list()}),
            "resources/list" => {
                json!({"resources":[{"uri":"chip://guide","name":"Chiptune engine guide","description":"Machine-readable engine and score format guide.","mimeType":"application/json"}]})
            }
            "resources/read" => {
                if params["uri"] == "chip://guide" {
                    json!({"contents":[{"uri":"chip://guide","mimeType":"application/json","text":serde_json::to_string(&crate::engine::describe())?}]})
                } else {
                    if let Some(id) = id {
                        writeln!(stdout, "{}", rpc_error(id, -32002, "Resource not found"))?;
                    }
                    continue;
                }
            }
            "tools/call" => {
                let n = params["name"].as_str().unwrap_or("");
                match tool_call(&root, n, &params["arguments"]) {
                    Ok(v) => {
                        json!({"content":[{"type":"text","text":serde_json::to_string(&v)?}],"structuredContent":v,"isError":false})
                    }
                    Err(e) => {
                        json!({"content":[{"type":"text","text":format!("{e:#}")}],"isError":true})
                    }
                }
            }
            _ => {
                if let Some(id) = id {
                    writeln!(stdout, "{}", rpc_error(id, -32601, "Method not found"))?;
                }
                continue;
            }
        };
        if let Some(id) = id {
            writeln!(
                stdout,
                "{}",
                json!({"jsonrpc":"2.0","id":id,"result":response})
            )?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_parent_and_absolute_paths() {
        let d = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&d).unwrap();
        let root = d.canonicalize().unwrap();
        assert!(inside(&root, "../outside").is_err());
        assert!(inside(&root, "C:/outside").is_err());
        let _ = fs::remove_dir_all(d);
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape_for_missing_child() {
        use std::os::unix::fs::symlink;
        let d = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let root = d.join("root");
        let outside = d.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, root.join("link")).unwrap();
        assert!(inside(&root.canonicalize().unwrap(), "link/new/file").is_err());
        let _ = fs::remove_dir_all(d);
    }
    #[test]
    fn rpc_unknown_method_is_method_not_found() {
        let r = rpc_error(json!(3), -32601, "Method not found");
        assert_eq!(r["error"]["code"], -32601);
    }
    #[test]
    fn existing_file_resolution_does_not_append_directory_separator() {
        let d=std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());fs::create_dir_all(&d).unwrap();
        fs::write(d.join("score.json"),b"{}").unwrap();
        let root=d.canonicalize().unwrap();let path=inside(&root,"score.json").unwrap();
        assert_eq!(fs::read(path).unwrap(),b"{}");
    }
}
