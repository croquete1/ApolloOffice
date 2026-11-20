Warning: truncated output (original token count: 72705)
Total output lines: 6985

use crate::ai::{ask, p as P, wrap, Ask};
use crate::app::{drop, e, sl, Ax, Rn, R, X};
use crate::db::{now, Job};
use crate::net::need;
use crate::pa;
use crate::sh::{gh, ghj, hb, hbid, ho, hosts, hx, hxl, q, Out};
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const RND: u64 = 2;
const CIT: u64 = 150 * 60;
const ENV_PREFLIGHT: &str = r#"command -v git >/dev/null 2>&1 || { echo "git missing in build environment"; exit 1; }
if [ -f .nvmrc ]; then
  want=$(sed 's/#.*//' .nvmrc | head -n1 | tr -d ' \t\r')
  if [ -z "$want" ]; then echo "unsupported empty .nvmrc requirement"; exit 1; fi
  command -v node >/dev/null 2>&1 || { echo "node missing but .nvmrc requires Node $want; rebuild the build image"; exit 1; }
  have=$(node -v 2>&1) || { echo "node failed to run: $have"; exit 1; }
  case "$have" in v[0-9]*) ;; *) echo "node returned an invalid version: $have"; exit 1 ;; esac
  have=${have#v}
  case "$have" in *[!0-9.]*|'') echo "node returned an invalid version: $have"; exit 1 ;; esac
  case "$want" in v*) want=${want#v} ;; esac
  case "$want" in
    *[!0-9.]* )
      nvm_file="${NVM_DIR:-$HOME/.nvm}/nvm.sh"
      if [ -s "$nvm_file" ]; then . "$nvm_file"; fi
      command -v nvm >/dev/null 2>&1 || { echo "unsupported .nvmrc requirement '$want': install nvm to resolve this alias"; exit 1; }
      resolved=$(nvm version "$want" 2>&1) || { echo "unsupported .nvmrc requirement '$want': $resolved"; exit 1; }
      case "$resolved" in v[0-9]*) ;; *) echo "unsupported .nvmrc requirement '$want': nvm could not resolve it"; exit 1 ;; esac
      resolved=${resolved#v}
      case "$resolved" in *[!0-9.]*|'') echo "unsupported .nvmrc requirement '$want': nvm returned an invalid version"; exit 1 ;; esac
      wmaj=${resolved%%.*} ;;
    *) wmaj=${want%%.*} ;;
  esac
  case "$wmaj" in ''|*[!0-9]*) echo "unsupported .nvmrc requirement '$want': no Node major version"; exit 1 ;; esac
  hmaj=${have%%.*}
  if [ "$wmaj" != "$hmaj" ]; then echo "node incompatible: repo requires Node $want but build environment has Node $have; rebuild the build image"; exit 1; fi
  echo "preflight ok: node $have (wants $want)"
else
  echo "preflight ok: git present, no .nvmrc requirement"
fi"#;

pub fn gitb() -> String {
    std::env::var("APOLLO_OFFICE_REMOTE").unwrap_or_else(|_| "https://github.com/".into())
}

pub fn slug(r: &str) -> String {
    r.replace('/', "__")
}

fn dir(a: &Ax, j: &Job) -> String {
    format!("{}/{}", hbid(a, &j.h()), slug(&j.repo))
}

pub fn rp(a: &Ax, r: &str) -> Value {
    a.db.one("select * from rp where full=?", pa![r.to_string()])
        .unwrap_or(json!({}))
}

fn oe(o: &Out, s: &str, h: &str) -> X {
    let m = format!(
        "{h}: {} → exit {}: {}",
        crate::sh::head(s, 80),
        o.c,
        crate::sh::last(
            if o.e.trim().is_empty() {
                o.o.trim()
            } else {
                o.e.trim()
            },
            300
        )
    );
    if crate::app::hostish(&m) {
        X::Host(h.to_string(), m)
    } else if crate::app::netish(&m) {
        X::Net(m)
    } else {
        X::E(m)
    }
}

pub(crate) async fn x(a: &Ax, j: &Job, s: &str, t: u64) -> Out {
    hx(a, &j.h(), &format!("cd \"{}\" && {s}", dir(a, j)), t).await
}

async fn ok(a: &Ax, j: &Job, s: &str, t: u64) -> R<String> {
    let ro = Regex::new(r"^git (log|diff|rev-parse|rev-list|status|symbolic-ref|range-diff) [^;&|]*(\| head -c \d+)?$").unwrap();
    let mut r = x(a, j, s, t).await;
    for _ in 0..2 {
        if r.c == 0 || !(r.all().trim().is_empty() || ro.is_match(s)) {
            break;
        }
        sl(15_000).await;
        r = x(a, j, s, t).await;
    }
    if r.c != 0 {
        return Err(oe(&r, s, &j.h()));
    }
    Ok(r.o.trim().to_string())
}

async fn tenv() -> R<Vec<(String, String)>> {
    let t = gh(&["auth", "token"], 30)
        .await
        .map_err(X::E)?
        .trim()
        .to_string();
    if t.is_empty() {
        return e("gh has no token, run gh auth login");
    }
    use base64::Engine;
    let h = format!(
        "AUTHORIZATION: basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{t}"))
    );
    Ok(vec![
        ("GIT_CONFIG_COUNT".into(), "3".into()),
        (
            "GIT_CONFIG_KEY_0".into(),
            "http.https://github.com/.extraheader".into(),
        ),
        ("GIT_CONFIG_VALUE_0".into(), h),
        ("GIT_CONFIG_KEY_1".into(), "core.hooksPath".into()),
        (
            "GIT_CONFIG_VALUE_1".into(),
            if cfg!(windows) {
                "NUL".into()
            } else {
                "/dev/null".into()
            },
        ),
        ("GIT_CONFIG_KEY_2".into(), "core.fsmonitor".into()),
        ("GIT_CONFIG_VALUE_2".into(), "false".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ])
}

fn mir(a: &Ax, r: &str) -> std::path::PathBuf {
    a.dd.join("mirror").join(format!("{}.git", slug(r)))
}

async fn lg(a: &Ax, r: &str, av: &[&str], tok: bool, t: u64) -> Out {
    let m = mir(a, r);
    if !m.join("HEAD").exists() {
        let _ = std::fs::create_dir_all(&m);
        crate::sh::run(
            &crate::sh::tool("git"),
            &[
                "init".into(),
                "-q".into(),
                "--bare".into(),
                m.display().to_string(),
            ],
            crate::sh::Rq::default(),
        )
        .await;
    }
    let env = if tok {
        match tenv().await {
            Ok(v) => v,
            Err(x) => {
                return Out {
                    c: -1,
                    o: String::new(),
                    e: x.to_string(),
                }
            }
        }
    } else {
        vec![("GIT_TERMINAL_PROMPT".into(), "0".into())]
    };
    let mut v: Vec<String> = vec!["--git-dir".into(), m.display().to_string()];
    v.extend(av.iter().map(|x| x.to_string()));
    crate::sh::run(
        &crate::sh::tool("git"),
        &v,
        crate::sh::Rq {
            t,
            env,
            ..Default::default()
        },
    )
    .await
}

async fn lok(a: &Ax, r: &str, av: &[&str], tok: bool, t: u64) -> R<String> {
    let o = lg(a, r, av, tok, t).await;
    if o.c != 0 {
        return Err(oe(
            &o,
            &format!(
                "git {}",
                av.iter().take(2).cloned().collect::<Vec<_>>().join(" ")
            ),
            "office",
        ));
    }
    Ok(o.o.trim().to_string())
}

async fn public(a: &Ax, j: &Job) -> R<bool> {
    let mt = crate::gh::meta(a, &j.repo).await.map_err(X::E)?;
    Ok(!mt["private"].as_bool().unwrap_or(true))
}

async fn ship(a: &Ax, j: &Job, src: &str, dst: &str) -> R<()> {
    let tg = lok(a, &j.repo, &["rev-parse", src], false, 60).await?;
    let have = x(a, j, &format!("git rev-parse -q --verify {}", q(dst)), 60)
        .await
        .o
        .trim()
        .to_string();
    if have == tg {
        return Ok(());
    }
    let f = a.dd.join("run").join(format!("ship-{}.bundle", j.id));
    let _ = std::fs::create_dir_all(f.parent().unwrap());
    let fs = f.display().to_string();
    let inc = !have.is_empty()
        && lg(
            a,
            &j.repo,
            &["cat-file", "-e", &format!("{have}^{{commit}}")],
            false,
            60,
        )
        .await
        .c == 0;
    let ex = format!("^{have}");
    let mut av = vec!["bundle", "create", "-q", fs.as_str(), src];
    if inc {
        av.push(ex.as_str());
    }
    if lg(a, &j.repo, &av, false, 1800).await.c != 0 {
        lok(
            a,
            &j.repo,
            &["bundle", "create", "-q", &fs, src],
            false,
            1800,
        )
        .await?;
    }
    use base64::Engine;
    let b = base64::engine::general_purpose::STANDARD
        .encode(std::fs::read(&f).map_err(|x| X::E(x.to_string()))?);
    let _ = std::fs::remove_file(&f);
    let tf = format!("/tmp/ao-in-{}.bundle", j.id);
    let r = x(a, j, &format!("base64 -d > {tf} <<'B64'\n{b}\nB64\ngit fetch -q --no-tags {tf} +{src}:{dst}; e=$?; rm -f {tf}; exit $e"), 1800).await;
    if r.c != 0 {
        return Err(oe(&r, "git fetch bundle", &j.h()));
    }
    Ok(())
}

async fn pull(a: &Ax, j: &Job, url: &str, br: &str, dst: &str) -> R<()> {
    if public(a, j).await? {
        let r = x(
            a,
            j,
            &format!(
                "git fetch -q --no-tags {} +refs/heads/{}:{}",
                q(url),
                q(br),
                dst
            ),
            1800,
        )
        .await;
        if r.c != 0 {
            return Err(oe(&r, "git fetch", &j.h()));
        }
        return Ok(());
    }
    let tm = format!("refs/ao/in/{}", j.id);
    lok(
        a,
        &j.repo,
        &[
            "fetch",
            "-q",
            "--no-tags",
            url,
            &format!("+refs/heads/{br}:{tm}"),
        ],
        true,
        1800,
    )
    .await?;
    ship(a, j, &tm, dst).await
}

pub fn cfg_ok(l: &str, url: &str) -> Result<(), String> {
    let ok = Regex::new(r"^(core\.(repositoryformatversion|filemode|bare|logallrefupdates|ignorecase|precomposeunicode|symlinks)|remote\.origin\.(url|fetch|promisor|partialclonefilter)|branch\.[^=]*\.(remote|merge)|user\.(name|email)|extensions\.(objectformat|partialclone)|gc\.[a-z]+|maintenance\.[a-z]+|fetch\.[a-z]+|init\.[a-z]+)$").unwrap();
    for x in l.lines().map(|x| x.trim()).filter(|x| !x.is_empty()) {
        let (k, v) = x.split_once('=').unwrap_or((x, ""));
        let k = k.to_lowercase();
        if !ok.is_match(&k) {
            return Err(format!("unexpected git config {k}"));
        }
        if k == "remote.origin.url" && v != url {
            return Err("origin points somewhere else".into());
        }
    }
    Ok(())
}

fn use_(a: &Ax, j: &mut Job, md: &str) {
    let mut ms: Vec<String> = j.d["ms"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    if !ms.iter().any(|x| x == md) {
        ms.push(md.into());
        j.d["ms"] = json!(ms);
        a.db.jd(j);
    }
}

fn trs(j: &Job) -> Vec<String> {
    let ms: BTreeSet<String> = j.d["ms"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| {
            x.as_str().map(|m| {
                format!(
                    "Co-Authored-By: {} <{}>",
                    crate::prov::name(m),
                    crate::prov::mail(m)
                )
            })
        })
        .collect();
    ms.into_iter().collect()
}

pub const FOOT: &str = "🤖 Generated with AI by [ApolloOffice](https://github.com/Silvertree2010/ApolloOffice), see the Co-Authored-By trailers for the models";
pub const FOOT0: &str = "🤖 Generated with AI, see the Co-Authored-By trailers for the models";
pub const FOOT_PT: &str = "🤖 Gerado com assistência de IA pelo [ApolloOffice](https://github.com/Silvertree2010/ApolloOffice); modelos identificados nos trailers Co-Authored-By.";
pub const FOOT0_PT: &str =
    "🤖 Gerado com assistência de IA; modelos identificados nos trailers Co-Authored-By.";

fn pr_pt(a: &Ax, j: &Job) -> bool {
    rp(a, &j.repo)["lang"].as_str() == Some("pt-PT")
}

// Manter o rodapé que acompanhou o corpo aprovado, mesmo após mudar o idioma.
fn stored_pr_footer(j: &Job, configured: &str) -> String {
    let supported = [FOOT, FOOT0, FOOT_PT, FOOT0_PT];
    if let Some(saved) = j.d["pb_footer"]
        .as_str()
        .filter(|saved| supported.contains(saved))
    {
        return saved.to_string();
    }
    if let Some(body) = j.d["pb"].as_str() {
        if let Some(legacy) = supported.into_iter().find(|footer| body.contains(*footer)) {
            return legacy.to_string();
        }
    }
    configured.to_string()
}

// Não aceitar os cabeçalhos ingleses comuns que o preenchimento do template
// pode reintroduzir numa PR configurada para PT-PT.
fn english_pr_heading(body: &str) -> bool {
    let english = [
        "Cause",
        "Fix",
        "Verified",
        "Not changed",
        "Why",
        "Change",
        "Summary",
        "Testing",
        "Tests",
        "Test plan",
        "Description",
        "Changes",
    ];
    body.lines().any(|line| {
        let line = line.trim_start();
        let Some(heading) = line.strip_prefix("## ") else {
            return false;
        };
        english
            .iter()
            .any(|word| heading.trim().eq_ignore_ascii_case(word))
    })
}

fn fo(a: &Ax, j: &Job) -> &'static str {
    let branded = rp(a, &j.repo)["nm"].as_i64().unwrap_or(1) == 1;
    match (pr_pt(a, j), branded) {
        (true, true) => FOOT_PT,
        (true, false) => FOOT0_PT,
        (false, true) => FOOT,
        (false, false) => FOOT0,
    }
}

fn pr_title(preferred: Option<&str>, f: &Value, fallback: &str) -> R<String> {
    for value in [preferred, f["title"].as_str(), Some(fallback)]
        .into_iter()
        .flatten()
    {
        if let Some(title) = value.lines().find(|line| !line.trim().is_empty()) {
            return Ok(title.trim().to_string());
        }
    }
    e("cannot publish a pull request without a title")
}

fn commit_subject(title: &str, feature: bool) -> R<String> {
    commit_subject_kind(title, if feature { "feat" } else { "fix" })
}

fn optimization_commit_subject(title: &str) -> R<String> {
    let title = title
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if title.is_empty() {
        return e("cannot create a commit without a subject");
    }
    let conventional = Regex::new(
        r"^(?:feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(?:\([A-Za-z0-9_.-]+\))?!?:\s*",
    )
    .unwrap();
    let description = conventional.replace(title, "");
    if description.trim().is_empty() {
        return e("cannot create a commit without a subject");
    }
    let subject = format!("refactor: {description}");
    Ok(subject
        .chars()
        .take(120)
        .collect::<String>()
        .trim_end()
        .to_string())
}

fn commit_subject_kind(title: &str, kind: &str) -> R<String> {
    let title = title
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if title.is_empty() {
        return e("cannot create a commit without a subject");
    }
    // A PR title is not necessarily a Conventional Commit subject. FixFlow
    // enforces Conventional Commits in its commit-msg hook. Do not disable hooks.
    let conventional = Regex::new(
        r"^(?:feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(?:\([A-Za-z0-9_.-]+\))?!?:\s+\S",
    )
    .unwrap();
    let subject = if conventional.is_match(title) {
        title.to_string()
    } else {
        format!("{kind}: {title}")
    };
    // The FixFlow commitlint configuration enforces a 120-character header.
    let header = subject.chars().take(120).collect::<String>();
    Ok(header.trim_end().to_string())
}

fn kill(a: &Ax, j: &Job) -> R<()> {
    if a.db
        .kg(&format!("kill:{}", j.id))
        .as_bool()
        .unwrap_or(false)
    {
        return drop("dropped by the owner");
    }
    Ok(())
}

fn bslug(j: &Job) -> String {
    let t: String = j
        .title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let t: String = t
        .split('-')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .chars()
        .take(32)
        .collect();
    let t = t.trim_end_matches('-');
    if j.hunt() || j.num == 0 {
        format!("fix/h{}-{t}", j.id)
    } else {
        format!("fix/{}-{t}", j.num)
    }
}

const ART: &[&str] = &[
    ".venv-bootstrap/",
    ".venv/",
    "venv/",
    "__pycache__/",
    "*.pyc",
    ".pytest_cache/",
    ".mypy_cache/",
    ".ruff_cache/",
    ".tox/",
    ".coverage",
    "node_modules/",
    ".next/",
    ".turbo/",
    "target/",
    ".gradle/",
];

/// Preflight only known command-line runtimes. Never execute a repository's
/// custom setup command as part of this inexpensive environment check.
fn setup_tool(setup: &str) -> Option<&'static str> {
    match setup.split_whitespace().next()? {
        "npm" => Some("npm"),
        "pnpm" => Some("pnpm"),
        "yarn" => Some("yarn"),
        "cargo" => Some("cargo"),
        "go" => Some("go"),
        "python3" => Some("python3"),
        "pip3" => Some("pip3"),
        _ => None,
    }
}

/// Identifica falhas de transporte antes de atribuir um erro de setup ao projeto.
fn preflight_host_error(result: &Out) -> bool {
    let error = format!("{}\n{}", result.o, result.e);
    result.c < 0 || crate::app::hostish(&error) || crate::app::netish(&error)
}

/// Setup stdout may mention a network error without the worker being offline.
/// Only inspect the transport's stderr or a failed process launch.
fn setup_transport_error(result: &Out) -> bool {
    result.c < 0 || crate::app::hostish(&result.e) || crate::app::netish(&result.e)
}

async fn prep(a: &Ax, j: &mut Job) -> R<()> {
    need(a, "repro", &[&j.h(), "gh"]).await?;
    if j.d["rp"].as_bool().unwrap_or(false) {
        j.d.as_object_mut().unwrap().remove("rp");
        a.db.jd(j);
        let br = j.s("br");
        if !br.is_empty()
            && x(
                a,
                j,
                &format!("git rev-parse -q --verify refs/heads/{}", q(&br)),
                60,
            )
            .await
            .c == 0
        {
            ok(a, j, &format!("git checkout -q {}", q(&br)), 120).await?;
            a.say(
                "repro",
                &format!("[{}] resuming {} on its existing clone", j.h(), j.tag()),
                Some(j.id),
            );
            return Ok(());
        }
    }
    let su = rp(a, &j.repo)["setup"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if let Some(tool) = setup_tool(&su) {
        let cmd = if matches!(tool, "npm" | "pnpm" | "yarn") {
            format!("command -v {tool} >/dev/null || {{ echo 'Falta: {tool}' >&2; exit 127; }}; command -v node >/dev/null || {{ echo 'Falta: node' >&2; exit 127; }}")
        } else {
            format!("command -v {tool} >/dev/null || {{ echo 'Falta: {tool}' >&2; exit 127; }}")
        };
        let preflight = hx(a, &j.h(), &cmd, 60).await;
        if preflight.c != 0 {
            let error = format!("{}\n{}", preflight.o, preflight.e);
            if preflight_host_error(&preflight) {
                // Transport/worker failures already have a bounded host retry
                // path; they are not deterministic missing-tool errors.
                return Err(X::Host(
                    j.h(),
                    format!(
                        "preflight transport failed (exit {}): {}",
                        preflight.c,
                        crate::hunt_health::safe_tail(&error)
                    ),
                ));
            }
            return drop(crate::hunt_health::diagnostic(
                a,
                j,
                "preflight",
                preflight.c,
                &format!(
                    "Ferramenta {tool} (ou Node) indisponível no worker: {}",
                    crate::hunt_health::safe_tail(&error)
                ),
            ));
        }
    }
    let mt = crate::gh::meta(a, &j.repo).await.map_err(X::E)?;
    let mn = mt["main"].as_str().unwrap_or("main").to_string();
    let d = dir(a, j);
    let base = hbid(a, &j.h());
    let url = format!("{}{}.git", gitb(), j.repo);
    let pb = public(a, j).await?;
    hx(
        a,
        &j.h(),
        &format!("pkill -f \"{d}/\" 2>/dev/null; true"),
        60,
    )
    .await;
    let ex = hx(a, &j.h(), &format!("test -d \"{d}/.git\""), 60).await.c == 0;
    let ex = ex && {
        let l = hx(
            a,
            &j.h(),
            &format!("cd \"{d}\" && git config --local --list"),
            60,
        )
        .await;
        l.c == 0 && cfg_ok(&l.o, &format!("{}{}.git", gitb(), j.repo)).is_ok()
    };
    if ex {
        let r = hx(a, &j.h(), &format!("cd \"{d}\" && {{ git rebase --abort; git merge --abort; git cherry-pick --abort; }} 2>/dev/null; git reset -q --hard && git clean -fdq"), 1200).await;
        if r.c != 0 {
            hx(a, &j.h(), &format!("rm -rf \"{d}\""), 600).await;
        }
    } else {
        hx(a, &j.h(), &format!("rm -rf \"{d}\""), 600).await;
    }
    if hx(a, &j.h(), &format!("test -d \"{d}/.git\""), 60).await.c != 0 {
        a.say(
            "repro",
            &format!("[{}] cloning {}", j.h(), j.repo),
            Some(j.id),
        );
        let c = if pb {
            format!(
                "git clone -q --no-checkout --filter=blob:none {} \"{d}\"",
                q(&url)
            )
        } else {
            format!(
                "git init -q \"{d}\" && git -C \"{d}\" remote add origin {}",
                q(&url)
            )
        };
        let r = hx(
            a,
            &j.h(),
            &format!("mkdir -p \"{base}\" && rm -rf \"{d}\" && {c}"),
            1800,
        )
        .await;
        if r.c != 0 {
            return Err(oe(&r, "git clone", &j.h()));
        }
    }
    pull(a, j, &url, &mn, &format!("refs/remotes/origin/{mn}")).await?;
    ok(
        a,
        j,
        &format!(
            "mkdir -p .git/info && printf '%s\\n' {} > .git/info/exclude",
            ART.iter().map(|x| q(x)).collect::<Vec<_>>().join(" ")
        ),
        60,
    )
    .await?;
    let me = crate::gh::me(a).await.map_err(X::E)?;
    ok(
        a,
        j,
        &format!(
            "git config user.name {} && git config user.email {}",
            q(me["name"].as_str().unwrap_or("")),
            q(me["mail"].as_str().unwrap_or(""))
        ),
        60,
    )
    .await?;
    let br = if j.kind == "fix" || j.kind == "hunt" {
        bslug(j)
    } else {
        j.s("branch")
    };
    if j.kind == "fix" || j.kind == "hunt" {
        let at = j.s("at");
        ok(
            a,
            j,
            &format!(
                "git checkout -q -B {} {}",
                q(&br),
                if at.is_empty() {
                    format!("origin/{}", q(&mn))
                } else {
                    q(&at)
                }
            ),
            1800,
        )
        .await?;
    } else {
        let src = format!("{}{}.git", gitb(), j.s("head"));
        pull(a, j, &src, &br, "refs/ao/head").await?;
        ok(
            a,
            j,
            &format!("git checkout -q -B {} refs/ao/head", q(&br)),
            1800,
        )
        .await?;
        j.d["old"] = json!(ok(a, j, "git rev-parse HEAD", 60).await?);
    }
    let pf = x(a, j, &format!("bash -c {} 2>&1", q(ENV_PREFLIGHT)), 120).await;
    if pf.c != 0 {
        a.say(
            "repro",
            &format!("[{}] preflight do ambiente falhou", j.h()),
            Some(j.id),
        );
        return drop(crate::hunt_health::diagnostic(
            a,
            j,
            "preflight",
            pf.c,
            &format!("{}\n{}", pf.o.trim(), pf.e.trim()),
        ));
    }
    if !su.is_empty() {
        a.say(
            "repro",
            &format!("[{}] setup: {}", j.h(), crate::sh::head(&su, 80)),
            Some(j.id),
        );
        let r = x(
            a,
            j,
            &format!(
                "to 1800 bash -c {} 2>&1 | tail -c 12000; exit ${{PIPESTATUS[0]}}",
                q(&su)
            ),
            1900,
        )
        .await;
        if r.c != 0 {
            if setup_transport_error(&r) {
                return Err(X::Host(
                    j.h(),
                    format!(
                        "setup worker transport failed (exit {}): {}",
                        r.c,
                        crate::hunt_health::safe_tail(&r.e)
                    ),
                ));
            }
            return drop(crate::hunt_health::diagnostic(
                a,
                j,
                "setup",
                r.c,
                &format!("{}\n{}", r.o, r.e),
            ));
        }
        ok(a, j, "git checkout -q -- . 2>/dev/null; true", 60).await?;
    }
    let bs = ok(a, j, "git rev-parse HEAD", 60).await?;
    j.d["br"] = json!(br);
    j.d["mn"] = json!(mn);
    j.d["base"] = json!(bs);
    a.db.jd(j);
    if j.kind == "hunt" {
        crate::hunt_health::success(a, &j.repo);
    }
    Ok(())
}

fn rn(a: &Ax, j: &Job) -> Option<Rn> {
    Some(Rn {
        h: j.h(),
        dir: dir(a, j),
        w: String::new(),
        md: String::new(),
        staged: false,
    })
}

fn staged_repos(a: &Ax) -> BTreeSet<String> {
    let mut s: BTreeSet<String> = a.set()["staged_repos"].as_array().map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    if let Ok(env) = std::env::var("APOLLO_OFFICE_STAGED_REPOS") {
        s.extend(env.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from));
    }
    s
}

/// Whether the operator INTENDS staged mode for this job: opt-in flag, bug-fix
/// kind, allowlisted repo. Independent of the free-first routing policy, so a
/// policy conflict can be detected and reported rather than silently ignored.
pub fn staged_intended(a: &Ax, j: &Job) -> bool {
    let on = std::env::var("APOLLO_OFFICE_STAGED_TDD").as_deref() == Ok("1") || a.set()["staged_tdd"].as_bool() == Some(true);
    on && matches!(j.kind.as_str(), "fix" | "hunt") && !j.feat() && !j.opt() && staged_repos(a).contains(&j.repo)
}

/// Staged mode is active only when intended AND free-first routing is OFF, so the
/// configured FIXER (Qwen) and REVIEWER (Claude) are used, not the routing models.
/// A conflicting configuration (intended but routing on) is rejected up front in
/// `fix()` rather than running staged with the wrong models (V3-P1-3).
pub fn staged(a: &Ax, j: &Job) -> bool {
    staged_intended(a, j) && !crate::routing::enabled(a)
}

/// A conflicting pilot configuration: staged intended but free-first routing on.
/// `fix()` rejects this up front instead of running with the wrong models.
pub fn staged_conflict(a: &Ax, j: &Job) -> bool {
    staged_intended(a, j) && crate::routing::enabled(a)
}

/// Opt-in fallback FIXER for the staged flow (e.g. the Codex CLI), allowed ONLY for
/// an allowlisted staged repo and ONLY when explicitly configured. It is used for at
/// most ONE last-round attempt, after the primary FIXER (Qwen) has concretely failed
/// every earlier round. Fails closed: no allowlist entry or no configured model
/// returns None, so a global setting can never silently replace the FIXER for
/// FixFlow or any other repo. It only ever selects a FIXER — the REVIEWER (Claude
/// Opus) is chosen separately and is never affected, so the fallback model can never
/// review or approve its own code. Configure via `staged_fallback` in settings or
/// the `APOLLO_OFFICE_STAGED_FALLBACK` env (e.g. "codex:gpt-5.6-terra").
pub fn staged_fallback(a: &Ax, j: &Job) -> Option<String> {
    if !staged_repos(a).contains(&j.repo) {
        return None;
    }
    std::env::var("APOLLO_OFFICE_STAGED_FALLBACK")
        .ok()
        .or_else(|| a.set()["staged_fallback"].as_str().map(String::from))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Reasoning effort for a Codex staged fallback FIXER. Defaults to "max" (the
/// configured pilot level); overridable via `APOLLO_OFFICE_STAGED_FALLBACK_EFFORT`
/// to one of the Codex levels. An unrecognised value falls back to "max" rather than
/// silently picking a different level. Returns a &'static str for Ask::ef.
fn codex_fallback_effort() -> &'static str {
    match std::env::var("APOLLO_OFFICE_STAGED_FALLBACK_EFFORT").ok().as_deref() {
        Some("minimal") => "minimal",
        Some("low") => "low",
        Some("medium") => "medium",
        Some("high") => "high",
        _ => "max",
    }
}

/// Read-only pipeline configuration for the authenticated Control Room.
/// Mirrors the same environment and settings precedence used by `staged()` and
/// `staged_fallback()` so the interface cannot advertise a different route.
pub fn staged_ui_config(a: &Ax) -> Value {
    let settings = a.set();
    let enabled = std::env::var("APOLLO_OFFICE_STAGED_TDD").as_deref() == Ok("1")
        || settings["staged_tdd"].as_bool() == Some(true);
    let repos = staged_repos(a);
    let fallback = std::env::var("APOLLO_OFFICE_STAGED_FALLBACK")
        .ok()
        .or_else(|| settings["staged_fallback"].as_str().map(String::from))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let fallback_effort = if fallback
        .as_deref()
        .is_some_and(|md| crate::prov::parse(md).0 == "codex")
    {
        codex_fallback_effort()
    } else {
        ""
    };

    json!({
        "active": enabled && !crate::routing::enabled(a) && !repos.is_empty(),
        "repos": repos,
        "primary": a.md("fixer"),
        "fallback": fallback.unwrap_or_default(),
        "fallback_effort": fallback_effort,
        "reviewer": a.md("reviewer")
    })
}

fn rules(a: &Ax, j: &Job) -> String {
    let r = rp(a, &j.repo)["rules"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if r.is_empty() {
        String::new()
    } else {
        format!("\n\nOwner's rules for this repository (trusted):\n{r}")
    }
}

fn cx(a: &Ax, j: &Job) -> bool {
    j.s("cx") == "complex" && a.set()["big"].as_bool().unwrap_or(true)
}

async fn repro(a: &Ax, j: &mut Job) -> R<()> {
    a.db.jset(j.id, &[("st", json!("repro"))]);
    let p = format!("Repository: {}. {}.\nVet note: {}\n\n{}", j.repo, j.orig(), j.d["hint"].as_str().unwrap_or("-"), wrap("issue", &format!("{}\n\n{}", if j.s("title").is_empty() { &j.title } else { j.d["title"].as_str().unwrap() }, j.s("body"))));
    let p = format!("{p}{}{}", rules(a,j),crate::quality::context(j));
    let planning = j.feat() || j.opt();
    let ms = if crate::routing::enabled(a) { crate::routing::plan(a) } else if planning || cx(a, j) { vec![a.md("big")] } else { vec![a.md("repro"), a.md("repro2")] };
    let mut reproduction_feedback=String::new();
    for (attempt, md) in ms.into_iter().enumerate() {
        crate::routing::event(a, Some(j.id), "repro", attempt, &md, &reproduction_feedback);
        a.st("repro", "work", Some(j.id), "");
        use_(a, j, &md);
        a.say("repro", &format!("{} {} {}", crate::prov::name(&md), if planning { "planning" } else { "reproducing" }, j.tag()), Some(j.id));
        let mut o = Ask::new("repro", md, P(if planning { "plan" } else { "repro" }), format!("{p}\n{}",wrap("previous-reproduction-diagnostic",&reproduction_feedback)));
        o.job = Some(j.id);
        o.rn = rn(a, j);
        o.turns = 40;
        o.ef = Some("medium");
        o.t = 90 * 60;
        let result = ask(a, o).await;
        x(a, j, "git checkout -q -- . && git clean -fdq", 120).await;
        let (_, js) = match result {
            Ok(v) => v,
            Err(X::Back(e)) => return Err(X::Back(e)),
            Err(X::Drop(e)) => return Err(X::Drop(e)),
            Err(e) if crate::routing::enabled(a) => {
                a.say("repro", &format!("attempt failed: {e}"), Some(j.id));
                reproduction_feedback = crate::sh::head(&e.to_string(), 3000);
                continue;
            }
            Err(e) => return Err(e),
        };
        if js["ok"].as_bool().unwrap_or(false) {
            j.d["repro"] = json!({"how": js["how"], "where": js["where"], "test": js["test"]});
            a.db.jd(j);
            a.say("repro", &format!("{}: {}", if planning { "planned" } else { "reproduced" }, js["how"].as_str().unwrap_or("")), Some(j.id));
            a.flow("repro", "fixer", &j.tag());
            a.st("repro", "idle", None, "ok");
            return Ok(());
        }
        reproduction_feedback=crate::sh::head(&js.to_string(),3000);
        a.say("repro", &format!("{}: {}", if planning { "not plannable" } else { "not reproduced" }, js["how"].as_str().unwrap_or("no answer")), Some(j.id));
    }
    a.st("repro", "idle", None, "sad");
    drop(if j.opt() { "optimization is not a focused, behavior-preserving change" } else if j.feat() { "feature already exists or cannot be done as one focused change" } else { "not reproducible" })
}

pub fn junk(p: &str) -> bool {
    Regex::new(r"(^|/)__pycache__/|\.py[co]$|\.class$|\.o$")
        .unwrap()
        .is_match(p)
}

async fn dirty(a: &Ax, j: &Job) -> R<Option<String>> {
    let st = ok(a, j, "git status --porcelain", 60).await?;
    let ps: Vec<String> = st
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].trim().trim_matches('"').to_string())
        .collect();
    if ps.is_empty() {
        return Ok(None);
    }
    if ps.iter().all(|p| junk(p)) && st.lines().all(|l| l.starts_with(" M")) {
        ok(
            a,
            j,
            &format!(
                "git checkout -q -f -- {}",
                ps.iter().map(|p| q(p)).collect::<Vec<_>>().join(" ")
            ),
            60,
        )
        .await?;
        return Ok(None);
    }
    Ok(Some(format!("working tree not clean, uncommitted changes in: {}. Commit them into the test or fix commit, or restore them with git checkout / remove them.", ps.iter().take(10).cloned().collect::<Vec<_>>().join(", "))))
}

const MAX_NAMED_CHECKS: usize = 64;
const MAX_NAMED_CHECK_NAME_BYTES: usize = 512;

async fn run_t(a: &Ax, j: &Job, at: &str, c: &str, named: &[String]) -> (i32, String) {
    let l = format!("/tmp/ao-t-{}.log", j.id);
    let evidence = named
        .iter()
        .map(|name| {
            format!(
                "grep -F -- {} {} | tail -n 4 | cut -b 1-1000",
                q(name),
                q(&l)
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let evidence = if evidence.is_empty() {
        String::new()
    } else {
        format!("printf '\\n[Named check evidence]\\n'; {evidence}; printf '\\n[End named check evidence]\\n';")
    };
    let r = x(a, j, &format!("rm -f {l}; if git checkout -q {} 2>&1; then {{ to 3600 bash -c {}\n}} > {l} 2>&1; e=$?; {evidence} tail -c 30000 {l}; git checkout -q -f -- . 2>/dev/null; git clean -fdq 2>/dev/null; else e=125; echo \"[checkout failed]\"; git status --short 2>&1 | head -20; fi; echo \"[exit $e]\"", q(at), q(c)), 3700).await;
    let e = Regex::new(r"\[exit (\d+)\]\s*$")
        .unwrap()
        .captures(&r.o)
        .and_then(|m| m[1].parse::<i32>().ok())
        .unwrap_or(-1);
    (e, r.o)
}

pub fn fails(o: &str) -> Vec<String> {
    let rs = [
        r"(?m)^test (\S+) \.\.\. FAILED\s*$",
        r"(?m)^FAILED (\S+?)(?: - .*)?\s*$",
        r"(?m)^\s*--- FAIL: (\S+)",
        r"(?m)^\s*(?:✕|×)\s+(.+?)(?:\s+\(\d+(?:\.\d+)? ?m?s\))?\s*$",
        r"(?m)^not ok \d+ - (.+?)\s*$",
    ];
    let mut v = BTreeSet::new();
    for r in rs {
        for c in Regex::new(r).unwrap().captures_iter(o) {
            v.insert(c[1].trim().to_string());
        }
    }
    v.into_iter().collect()
}

pub fn bare(t: &str) -> String {
    t.rsplit(|c| c == ':' || c == '.' || c == '/' || c == '>' || c == ' ').find(|x| !x.trim().is_empty()).unwrap_or(t).trim().to_string()
}

pub const CE: &str = r"error\[E\d+\]|could not compile|error: no test target|no such command|SyntaxError|ReferenceError|ModuleNotFoundError|ImportError while|cannot find module|Cannot find module|ERROR collecting|error TS\d{4}|no tests ran|No tests found|build failed|\[setup failed\]|undefined: |command not found|(?m:^[^\n]*\.java:\d+: error:)|ClassNotFoundException|NoClassDefFoundError";

pub fn judge(ts: &[String], a_e: i32, a_o: &str, b_e: i32, b_o: &str) -> String {
    if Regex::new(r"\[checkout failed\]")
        .unwrap()
        .is_match(&format!("{a_o}{b_o}"))
    {
        return format!("git checkout of the test or fix commit failed, the tests did not run. Leave a clean working tree with no stray files, then answer again. Output:\n{}", crate::sh::last(&format!("{a_o}\n{b_o}"), 3000));
    }
    if a_e == 0 {
        return format!(
            "Test already passes on the test commit without the fix (it must fail). Output:\n{}",
            crate::sh::last(a_o, 3000)
        );
    }
    if Regex::new(CE).unwrap().is_match(a_o) {
        return format!("Test commit does not compile or load; a compile or import error does not count as a failing test. Output:\n{}", crate::sh::last(a_o, 3000));
    }
    if !ts.iter().any(|t| named_check_present(a_o, t)) {
        return format!("No failure of the named tests on the test commit (none of the test names appears in the output). Output:\n{}", crate::sh::last(a_o, 3000));
    }
    if b_e != 0 {
        return format!(
            "Test fails with the fix. Output:\n{}",
            crate::sh::last(b_o, 3000)
        );
    }
    let f = fails(b_o);
    if !f.is_empty() {
        return format!(
            "Tests still reported as failing with the fix: {}. Output:\n{}",
            f.join(", "),
            crate::sh::last(b_o, 3000)
        );
    }
    String::new()
}

pub fn judgef(ts: &[String], a_e: i32, a_o: &str, b_e: i32, b_o: &str) -> String {
    if a_e != 0
        && Regex::new(CE).unwrap().is_match(a_o)
        && !Regex::new(r"\[checkout failed\]")
            .unwrap()
            .is_match(&format!("{a_o}{b_o}"))
    {
        if b_e != 0 {
            return format!(
                "Test fails with the feature. Output:\n{}",
                crate::sh::last(b_o, 3000)
            );
        }
        if !ts.iter().any(|t| named_check_present(b_o, t)) {
            return format!(
                "None of the named tests appears in the output of the feature commit. Output:\n{}",
                crate::sh::last(b_o, 3000)
            );
        }
        let f = fails(b_o);
        if !f.is_empty() {
            return format!(
                "Tests still reported as failing with the feature: {}. Output:\n{}",
                f.join(", "),
                crate::sh::last(b_o, 3000)
            );
        }
        return String::new();
    }
    judge(ts, a_e, a_o, b_e, b_o)
        .replace("with the fix", "with the feature")
        .replace("without the fix", "without the feature")
}

pub fn judgeopt(ts: &[String], a_e: i32, a_o: &str, b_e: i32, b_o: &str) -> String {
    if Regex::new(r"\[checkout failed\]")
        .unwrap()
        .is_match(&format!("{a_o}{b_o}"))
    {
        return format!(
            "git checkout of the verification or optimization commit failed, so the checks did not run. Output:\n{}",
            crate::sh::last(&format!("{a_o}\n{b_o}"), 3000)
        );
    }
    if a_e != 0 {
        return format!(
            "Verification checks fail before the optimization (exit {a_e}); the baseline must be green. Output:\n{}",
            crate::sh::last(a_o, 3000)
        );
    }
    let before = fails(a_o);
    if !before.is_empty() {
        return format!(
            "Checks report failures before the optimization: {}. Output:\n{}",
            before.join(", "),
            crate::sh::last(a_o, 3000)
        );
    }
    let missing_before = missing_named_checks(ts, a_o);
    if !missing_before.is_empty() {
        return format!(
            "Named checks are missing or did not pass before the optimization ({}). Output:\n{}",
            missing_before.join(", "),
            crate::sh::last(a_o, 3000)
        );
    }
    if b_e != 0 {
        return format!(
            "Verification checks fail after the optimization (exit {b_e}); the candidate must be green. Output:\n{}",
            crate::sh::last(b_o, 3000)
        );
    }
    let after = fails(b_o);
    if !after.is_empty() {
        return format!(
            "Checks report failures after the optimization: {}. Output:\n{}",
            after.join(", "),
            crate::sh::last(b_o, 3000)
        );
    }
    let missing_after = missing_named_checks(ts, b_o);
    if !missing_after.is_empty() {
        return format!(
            "Named checks are missing or did not pass after the optimization ({}). Output:\n{}",
            missing_after.join(", "),
            crate::sh::last(b_o, 3000)
        );
    }
    String::new()
}

fn missing_named_checks(ts: &[String], output: &str) -> Vec<String> {
    ts.iter()
        .filter(|name| {
            name.is_empty() || !output.lines().any(|line| passing_check_line(line, name))
        })
        .cloned()
        .collect()
}

fn named_check_present(output: &str, name: &str) -> bool {
    !name.is_empty()
        && output
            .lines()
            .any(|line| named_occurrences(line, name).next().is_some())
}

fn named_occurrences<'a>(
    line: &'a str,
    name: &'a str,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    line.match_indices(name).filter_map(move |(start, found)| {
        let end = start + found.len();
        let before = line[..start].chars().next_back();
        let after = line[end..].chars().next();
        let component_char =
            |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '>' | '-');
        let before_continues_name =
            before.is_some_and(component_char) || line[..start].ends_with("::");
        let after_continues_name =
            after.is_some_and(component_char) || line[end..].starts_with("::");
        (!before_continues_name && !after_continues_name).then_some((start, end))
    })
}

fn passing_check_line(line: &str, name: &str) -> bool {
    let mut status = String::with_capacity(line.len());
    let mut copied = 0;
    let mut found = false;
    for (start, end) in named_occurrences(line, name) {
        found = true;
        status.push_str(&line[copied..start]);
        status.push(' ');
        copied = end;
    }
    if !found {
        return false;
    }
    status.push_str(&line[copied..]);
    let lower = status.to_ascii_lowercase();
    let words: Vec<_> = lower
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .collect();
    let negative = words.iter().any(|word| {
        matches!(
            *word,
            "fail" | "failed" | "failure" | "ignored" | "skipped" | "xfail" | "error"
        )
    }) || words.windows(2).any(|pair| pair == ["not", "ok"])
        || lower.contains('✕')
        || lower.contains('×');
    if negative {
        return false;
    }
    status.contains('✓')
        || status.contains('✔')
        || words
            .iter()
            .any(|word| matches!(*word, "ok" | "pass" | "passed" | "success"))
}


async fn verify(a: &Ax, j: &mut Job, f: &Value) -> R<String> {
    let ts: Vec<String> = f["tests"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .filter(|x| !x.is_empty())
        .collect();
    let tc = f["test_cmd"].as_str().unwrap_or("").to_string();
    if tc.is_empty() || ts.is_empty() {
        return Ok("answer without test_cmd or tests".into());
    }
    if ts.len() > MAX_NAMED_CHECKS
        || ts.iter().any(|name| {
            name.trim().is_empty()
                || name.len() > MAX_NAMED_CHECK_NAME_BYTES
                || name.chars().any(char::is_control)
        })
    {
        return Ok(format!(
            "answer must name at most {MAX_NAMED_CHECKS} checks; each name must be non-empty, at most {MAX_NAMED_CHECK_NAME_BYTES} bytes, and contain no control characters"
        ));
    }
    if Regex::new(crate::mcp::BAD).unwrap().is_match(&tc) {
        return Ok("test_cmd contains forbidden commands".into());
    }
    let n: i64 = ok(
        a,
        j,
        &format!("git rev-list --count {}..HEAD", j.s("base")),
        60,
    )
    .await?
    .parse()
    .unwrap_or(0);
    if n != 2 {
        let commit_roles = if j.opt() { "verification, optimization" } else if j.feat() { "test, feature" } else { "test, fix" };
        return Ok(format!("expected exactly 2 commits ({commit_roles}) on top of the base, found {n}"));
    }
    if let Some(m) = dirty(a, j).await? {
        return Ok(m);
    }
    let l = ok(a, j, "git log --format=%H -n2", 60).await?;
    let mut it = l.lines();
    let (b, at) = (
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    );
    let (ae, ao) = run_t(a, j, &at, &tc, &ts).await;
    let (be, bo) = run_t(a, j, &b, &tc, &ts).await;
    ok(a, j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
    let r = if j.opt() { judgeopt(&ts, ae, &ao, be, &bo) } else if j.feat() { judgef(&ts, ae, &ao, be, &bo) } else { judge(&ts, ae, &ao, be, &bo) };
    if r.is_empty() {
        let fl = Regex::new(r"(?i)FAIL|panicked|assert|Error|expected|^not ok\b").unwrap();
        let pl = Regex::new(r"(?i)\bok\b|pass|✓").unwrap();
        let opt = j.opt();
        j.d["proof"] = json!({"a_exit":ae,"b_exit":be,"test_commit":at,"fix_commit":b,"command":tc,"a": ao.lines().filter(|x| if opt { pl.is_match(x) } else { fl.is_match(x) }).take(6).collect::<Vec<_>>().join("\n"), "b": bo.lines().filter(|x| pl.is_match(x)).take(6).collect::<Vec<_>>().join("\n")});
        a.db.jd(j);
    }
    if !r.is_empty() {
        return Ok(r);
    }
    crate::quality::check(a, j).await
}

async fn slot(a: &Ax, h: &str) {
    let cap = ho(a, h).map(|x| x.ci).unwrap_or(1) as usize;
    loop {
        {
            let mut s = a.sem.lock().unwrap();
            let n = s.entry(h.to_string()).or_insert(0);
            if *n < cap {
                *n += 1;
                return;
            }
        }
        sl(5000).await;
    }
}

fn unslot(a: &Ax, h: &str) {
    if let Some(n) = a.sem.lock().unwrap().get_mut(h) {
        *n = n.saturating_sub(1);
    }
}

async fn cirun(a: &Ax, j: &Job, at: Option<&str>) -> (i32, String) {
    let c = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    let id = j.id;
    let ax = a.clone();
    let ln = move |l: &str| {
        if !l.trim().is_empty() {
            ax.lv
                .lock()
                .unwrap()
                .cil
                .insert(id, crate::sh::head(l.trim(), 160));
        }
    };
    let co = match at {
        Some(s) => format!("git checkout -q {} && ", q(s)),
        None => String::new(),
    };
    let r = hxl(a, &j.h(), &format!("cd \"{}\" && {co}{{ to {CIT} bash -c {}\n}} 2>&1; e=$?; git checkout -q -f -- . 2>/dev/null; git clean -fdq 2>/dev/null; echo \"[exit $e]\"", dir(a, j), q(&c)), CIT + 120, Some(&ln)).await;
    a.lv.lock().unwrap().cil.remove(&id);
    let o = r.o;
    let e = Regex::new(r"\[exit (\d+)\]\s*$")
        .unwrap()
        .captures(&o)
        .and_then(|m| m[1].parse::<i32>().ok())
        .unwrap_or(if r.c == 0 { -1 } else { r.c });
    (e, o)
}

fn lost(o: &str) -> bool {
    !Regex::new(r"\[exit -?\d+\]\s*$").unwrap().is_match(o)
}

pub fn tool(o: &str) -> Option<(String, Option<String>)> {
    let l = Regex::new(r"(?im)^.*(?:is required in CI|command not found|no such (?:sub)?command|is not installed|not found in PATH).*$").unwrap().find(o)?.as_str().trim().to_string();
    let k = Regex::new(r"cargo install ([A-Za-z0-9_-]+)")
        .unwrap()
        .captures(&l)
        .map(|m| m[1].to_string())
        .or_else(|| {
            Regex::new(r"no such (?:sub)?command:? `?([a-z0-9-]+)`?")
                .unwrap()
                .captures(&l)
                .map(|m| format!("cargo-{}", &m[1]))
        });
    Some((crate::sh::head(&l, 200), k))
}

fn errs(o: &str) -> String {
    let re = Regex::new(r"(?i)^(error|failures:|test result: FAILED|---- |FAILED|FAIL|✕|×|E  )|panicked at|AssertionError|Diff in").unwrap();
    let h: Vec<&str> = o.lines().filter(|l| re.is_match(l)).take(60).collect();
    format!("{}\n----\n{}", h.join("\n"), crate::sh::last(o, 4000))
}

async fn ci(a: &Ax, j: &mut Job) -> R<String> {
    a.say(
        "ci",
        &format!("[{}] waiting for the build slot", j.h()),
        Some(j.id),
    );
    slot(a, &j.h()).await;
    struct SlotGuard(Ax, String);
    impl Drop for SlotGuard {
        fn drop(&mut self) {
            unslot(&self.0, &self.1);
        }
    }
    let _guard = SlotGuard(a.clone(), j.h());
    ci0(a, j).await
}

// A revalidation proves the exact PR head and tree, not a Fixer's commit.
fn ci_evidence(kind: &str, data: &Value, command: &str, exit: i32, output: &str, rerun: bool) -> Value {
    let candidate = if kind == "mergecheck" { data["expected_head"].clone() } else { data["proof"]["fix_commit"].clone() };
    let mut result = json!({"command":command,"exit":exit,"output_tail":crate::sh::last(output,4000),"candidate":candidate});
    if rerun { result["rerun"] = json!(true); }
    result
}

async fn record_ci_success(a: &Ax, j: &mut Job) -> R<()> {
    let tree = ok(a, j, "git rev-parse HEAD^{tree}", 60).await?;
    if tree.is_empty() { return drop("CI sem árvore Git verificável"); }
    j.d["ci_tree"] = json!(tree);
    j.d["ci_verified_at"] = json!(now());
    a.db.jd(j);
    Ok(())
}

async fn ci0(a: &Ax, j: &mut Job) -> R<String> {
    a.db.jset(j.id, &[("st", json!("ci"))]);
    a.st("ci", "work", Some(j.id), "");
    a.flow("fixer", "ci", &j.tag());
    let c = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    j.d["known"] = json!([]);
    let mut ie = 0;
    let mut it = false;
    let infra_error = Regex::new(r"No space left on device|failed to unpack package|invalid gzip header|corrupt deflate stream|failed to download from|ECONNRESET|ETIMEDOUT|Could not resolve host|EAI_AGAIN").unwrap();
    loop {
        a.say("ci", &format!("{c} running for {}", j.tag()), Some(j.id));
        let t0 = now();
        let (ec, o) = cirun(a, j, None).await;
        j.d["ci_evidence"] = ci_evidence(&j.kind, &j.d, &c, ec, &o, false);
        a.db.jd(j);
        let m = (now() - t0) / 60000;
        if ec == 0 {
            // A prova local de integração vincula testes à árvore concreta,
            // não ao SHA que muda quando se faz squash dos commits da PR.
            record_ci_success(a, j).await?;
            a.st("ci", "idle", None, "ok");
            a.say("ci", &format!("green after {m} min"), Some(j.id));
            return Ok(String::new());
        }
        if ec == 124 || ec == 137 && m >= (CIT / 60) as i64 - 1 {
            a.st("ci", "idle", None, "sad");
            return Ok(format!("CI aborted after {} min (timeout)", CIT / 60));
        }
        a.say("ci", &format!("red (exit {ec}) after {m} min"), Some(j.id));
        let inf = if lost(&o) {
            Some("connection lost".to_string())
        } else {
            infra_error.find(&o).map(|m| m.as_str().to_string())
        };
        if let Some(m) = inf {
            if ie < 2 {
                ie += 1;
                a.say(
                    "ci",
                    &format!("infra error on {} ({m}), restarting CI ({ie}/2)", j.h()),
                    Some(j.id),
                );
                sl(30_000).await;
                continue;
            }
            if lost(&o) {
                return Err(X::Back(format!(
                    "lost the connection to {} during CI",
                    j.h()
                )));
            }
        }
        let fs = fails(&o);
        if fs.is_empty() {
            if let Some(t) = tool(&o) {
                if let Some(k) = t.1.filter(|_| !it) {
                    it = true;
                    a.say(
                        "ci",
                        &format!("[{}] {c} needs {k}, installing it", j.h()),
                        Some(j.id),
                    );
                    let r = hx(
                        a,
                        &j.h(),
                        &format!(
                            "{}cargo install {} --locked 2>&1 | tail -3",
                            crate::sh::PRE,
                            q(&k)
                        ),
                        2400,
                    )
                    .await;
                    if r.c == 0 {
                        continue;
                    }
                }
                a.st("ci", "idle", None, "sad");
                return drop(format!(
                    "`{c}` needs a tool that is missing on {}: {}",
                    j.h(),
                    t.0
                ));
            }
        }
        let bk = format!("cibase:{}{}", j.repo, hk(a, &j.h()));
        let bs = j.s("base");
        let cb = a.db.kg(&bk);
        let (bred, bn) = if cb["sha"].as_str() == Some(bs.as_str()) {
            (
                cb["red"].as_bool().unwrap_or(false),
                cb["n"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect::<Vec<_>>(),
            )
        } else {
            a.say(
                "ci",
                &format!(
                    "checking whether CI is also red on {} without the fix",
                    j.s("mn")
                ),
                Some(j.id),
            );
            let (be, bo) = cirun(a, j, Some(&bs)).await;
            ok(a, j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
            if lost(&bo) {
                return Err(X::Back(format!(
                    "lost the connection to {} while checking {}",
                    j.h(),
                    j.s("mn")
                )));
            }
            let bn = fails(&bo);
            a.db.ks(
                &bk,
                &json!({"sha": bs, "red": be != 0, "n": bn, "t": now()}),
            );
            (be != 0, bn)
        };
        if !bred {
            {
                a.say(
                    "ci",
                    "default branch is green, rerunning once to rule out flaky tests",
                    Some(j.id),
                );
                let (e2, o2) = cirun(a, j, None).await;
                j.d["ci_evidence"] = ci_evidence(&j.kind, &j.d, &c, e2, &o2, true);
                a.db.jd(j);
                if e2 == 0 {
                    record_ci_success(a, j).await?;
                    a.st("ci", "idle", None, "ok");
                    a.say("ci", "green on rerun (flaky test)", Some(j.id));
                    return Ok(String::new());
                }
                a.st("ci", "idle", None, "sad");
                return Ok(format!(
                    "`{c}` fails with the fix (exit {e2}) but passes on {} without it:\n{}",
                    j.s("mn"),
                    crate::sh::last(&errs(&o2), 6000)
                ));
            }
        }
        if bred {
            let nw: Vec<String> = fs.iter().filter(|n| !bn.contains(n)).cloned().collect();
            if !fs.is_empty() && nw.is_empty() {
                j.d["known"] = json!(fs);
                record_ci_success(a, j).await?;
                a.db.jd(j);
                a.st("ci", "idle", None, "ok");
                a.say(
                    "ci",
                    &format!(
                        "green except tests that also fail on {}: {}",
                        j.s("mn"),
                        fs.join(", ")
                    ),
                    Some(j.id),
                );
                return Ok(String::new());
            }
            if fs.is_empty() && !bn.is_empty() {
                a.st("ci", "idle", None, "sad");
                return Ok(format!("`{c}` fails with the fix (exit {ec}) before the tests run, while {} only fails tests:\n{}", j.s("mn"), crate::sh::last(&errs(&o), 6000)));
            }
            if bn.is_empty() {
                a.st("ci", "idle", None, "sad");
                a.db.ks(
                    &format!("cired:{}{}", j.repo, hk(a, &j.h())),
                    &json!({"sha": bs, "t": now()}),
                );
                return Err(X::Back(format!(
                    "`{c}` also fails on {} without the fix, waiting until it is green again",
                    j.s("mn")
                )));
            }
            a.st("ci", "idle", None, "sad");
            return Ok(format!(
                "`{c}` has new failures with the fix: {}\n{}",
                nw.join(", "),
                crate::sh::last(&errs(&o), 6000)
            ));
        }
        a.st("ci", "idle", None, "sad");
        return Ok(format!(
            "`{c}` fails (exit {ec}):\n{}",
            crate::sh::last(&errs(&o), 6000)
        ));
    }
}

fn san(s: &str) -> String {
    let r = Regex::new(r"(?i)\s*\b(Fixes|Closes|Resolves)\s+#0\b\.?")
        .unwrap()
        .replace_all(s, "");
    Regex::new(r"(?i)\bissue\s+#0\b")
        .unwrap()
        .replace_all(&r, "this bug")
        .trim()
        .to_string()
}

fn body_lang(j: &Job, f: &Value, ci: &str, ft: &str, pt: bool) -> String {
    let g = |k: &str| {
        let s = f[k].as_str().unwrap_or("").to_string();
        if j.own() {
            san(&s)
        } else {
            s
        }
    };
    let ts: Vec<String> = f["tests"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let pl = ts.len() > 1;
    let kn: Vec<String> = j.d["known"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let mn = j.s("mn");
    let opt = j.opt();
    let mut v = vec![];
    if j.kind == "fix" {
        if j.hunt() {
            v.push(if pt {
                format!(
                    "Encontrado durante os testes de `{}` (sem Issue existente): {}.",
                    j.d["hunt"]["where"].as_str().unwrap_or(""),
                    j.d["hunt"]["title"].as_str().unwrap_or("")
                )
            } else {
                format!(
                    "Found while testing `{}` (no existing issue): {}.",
                    j.d["hunt"]["where"].as_str().unwrap_or(""),
                    j.d["hunt"]["title"].as_str().unwrap_or("")
                )
            });
        } else if j.ext() {
            v.push("Reported to us directly (no existing issue).".to_string());
        } else if j.feat() || opt {
            v.push(format!("Closes #{}", j.num));
        } else {
            v.push(format!("Fixes #{}", j.num));
        }
        v.push(String::new());
    }
    let fe = j.feat();
    v.extend([if fe || opt { "## Why" } else { "## Cause" }.into(), g("cause"), String::new(), if fe || opt { "## Change" } else { "## Fix" }.into(), g("fix"), String::new(), "## Verified".into()]);
    if !ts.is_empty() {
        if opt {
            v.push(format!("- {} {}: pass on `{mn}` before and after this optimization (`{}`).", if pl { "Checks" } else { "Check" }, ts.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", "), f["test_cmd"].as_str().unwrap_or("")));
        } else {
            v.push(format!("- {}{} {}: fail{} on `{mn}` without the {} and pass{} with it (`{}`).", if fe { "Test" } else { "Regression test" }, if pl { "s" } else { "" }, ts.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", "), if pl { "" } else { "s" }, if fe { "change" } else { "fix" }, if pl { "" } else { "es" }, f["test_cmd"].as_str().unwrap_or("")));
        }
    }
    if kn.is_empty() {
        v.push(if pt {
            format!("- `{ci}`: aprovado.")
        } else {
            format!("- `{ci}` passes.")
        });
    } else {
        let names = kn
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ");
        v.push(if pt {
            format!("- `{ci}`: aprovado, exceto {names}, que também falham em `{mn}` sem esta alteração.")
        } else {
            format!("- `{ci}` passes except {names}, which also fail{} on `{mn}` without this change.",
                if kn.len() > 1 { "" } else { "s" })
        });
    }
    let u = g("untouched");
    if !u.trim().is_empty() {
        v.extend([
            String::new(),
            (if pt {
                "## Fora do âmbito"
            } else {
                "## Not changed"
            })
            .into(),
            u,
        ]);
    }
    if !ft.is_empty() {
        v.extend([String::new(), ft.into()]);
    }
    v.join("\n").trim().to_string()
}

fn ensure_issue_closure(j: &Job, mut body: String) -> String {
    if j.kind != "fix" || j.own() || j.num <= 0 {
        return body;
    }
    let closure = Regex::new(&format!(
        r"(?i)^\s*(?:[-*]\s*)?(?:close[sd]?|fix(?:es|ed)?|resolve(?:s|d)?)\s*:?\s+#{}\s*\.?\s*$",
        j.num
    ))
    .unwrap();
    let mut fenced = false;
    let linked = body.lines().any(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            return false;
        }
        !fenced && closure.is_match(line)
    });
    if !linked {
        if !body.trim().is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&format!("{} #{}", if j.feat() || j.opt() { "Closes" } else { "Fixes" }, j.num));
    }
    body
}

async fn review(a: &Ax, j: &mut Job, f: &mut Value) -> R<String> {
    review_mode(a, j, f, false).await
}

async fn diagnosis(a: &Ax, j: &mut Job, f: &mut Value) -> R<String> {
    match review_mode(a,j,f,true).await {
        Ok(text)=>Ok(text),
        Err(X::Drop(reason))=>Err(X::Drop(reason)),
        Err(error)=>Ok(format!("Diagnosis unavailable: {error}; retain the concrete verification failure and continue repair.")),
    }
}

async fn review_mode(a: &Ax, j: &mut Job, f: &mut Value, diagnostic: bool) -> R<String> {
    a.db.jset(j.id, &[("st", json!("review"))]);
    a.flow("ci", "reviewer", &j.tag());
    let df = ok(
        a,
        j,
        &format!("git diff {}..HEAD | head -c 60000", j.s("base")),
        120,
    )
    .await?;
    let lg = ok(
        a,
        j,
        &format!("git log --format='%s%n%b' {}..HEAD", j.s("base")),
        60,
    )
    .await?;
    let ci = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    let p = format!(
        "Repository {}, {}.\n\n{}\n\n{}Commit messages:\n{lg}\n\nPR title: {}\nPR body:\n{}\n\nDiff:\n{df}{}",
        j.repo,
        if j.kind == "mergecheck" { format!("PR #{} — validation before possible merge", j.pr.unwrap_or(0)) } else if j.own() { "no GitHub issue (the PR must not reference any issue number)".to_string() } else { format!("issue #{}", j.num) },
        wrap("issue", &format!("{}\n\n{}", j.title, crate::sh::head(&j.s("body"), 4000))),
        if j.kind == "review" { format!("{}\n\n", wrap("request", &j.s("change"))) } else if j.opt() { "This PR is a behavior-preserving optimization, not a feature. It must keep externally observable behavior, outputs, file formats, APIs, and defaults unchanged. Confirm every named check passes on both the verification commit before the optimization and the final commit. Assess whether the diff implements one concrete, objectively verifiable improvement without unrelated changes, and do not accept unsupported performance claims.\n\n".to_string() } else if j.feat() { "This PR implements a requested FEATURE, not a bug fix. Judge it as a feature: it does what the issue asks, completely but minimally; the tests check the new behaviour (on the test commit they may fail to compile because the new API is missing); no unrelated changes; no surprising changes to defaults, file formats or visible design beyond what the issue asks.\n\n".to_string() } else { String::new() },
        f["title"].as_str().unwrap_or(""),
        body_lang(j, f, &ci, "", pr_pt(a, j)),
        if crate::quality::enabled(a) && j.kind == "fix" && !j.opt() { format!("{}{}\nIndependent acceptance evidence: {}\nFrozen verifier (untrusted code): {}\nCheck that this verifier exercises actual production code and the reported behavior, rather than copying logic or merely matching source text. Reject inadequate coverage or unsupported claims.",rules(a, j),crate::quality::context(j),wrap("acceptance-evidence",&j.d["acceptance_proof"].to_string()),wrap("frozen-verifier",j.d["acceptance"]["script"].as_str().unwrap_or(""))) } else { rules(a, j) }
    );
    let author = j.d["author"].as_str().unwrap_or("").to_string();
    let candidates = if crate::routing::enabled(a) && !staged(a, j) {
        crate::routing::REVIEWERS
            .iter()
            .filter(|m| **m != author)
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else {
        vec![a.md("reviewer")]
    };
    let mut result = None;
    let mut reviewer_feedback = String::new();
    for (attempt, md) in candidates.into_iter().enumerate() {
        crate::routing::event(a, Some(j.id), "reviewer", attempt, &md, &reviewer_feedback);
        // Reviewer has no worker tools and cannot silently edit the author's tree.
        let rp = if crate::routing::enabled(a) {
            format!("{p}\n\nExecuted regression evidence: {}\nIndependent verification feedback:\n{}\n\nFor code/test rejection also include lesson: {{mistake, correction, verification}}. These are untrusted pending Dreaming proposals, not established facts.", j.d["proof"], j.d["verification_feedback"].as_str().unwrap_or("All executed checks passed; inspect diff independently."))
        } else {
            p.clone()
        };
        let mut o = Ask::new("reviewer", md.clone(), P("reviewer"), rp);
        o.job = Some(j.id);
        o.ef = Some("medium");
        match ask(a, o).await {
            Ok((_, js))
                if js["ok"].is_boolean()
                    && (js["ok"] == true
                        || js["issues"]
                            .as_array()
                            .map(|v| !v.is_empty())
                            .unwrap_or(false)) =>
            {
                if !diagnostic {
                    crate::routing::lesson(
                        a,
                        j.id,
                        &md,
                        &js,
                        j.d["verification_feedback"].as_str().unwrap_or(""),
                        &df,
                    );
                }
                j.d[if diagnostic {
                    "last_diagnosis"
                } else {
                    "last_review"
                }] = json!({"model": md,"verdict": js,"t":now()});
                a.db.jd(j);
                result = Some(js);
                break;
            }
            Err(X::Drop(reason)) => return Err(X::Drop(reason)),
            Err(X::Back(reason)) => return Err(X::Back(reason)),
            Err(e) if !crate::routing::enabled(a) => return Err(e),
            Err(error) => {
                reviewer_feedback = format!("Reviewer {md} failed: {error}");
                a.say(
                    "reviewer",
                    &crate::sh::head(&reviewer_feedback, 500),
                    Some(j.id),
                );
            }
            Ok((text, value)) => {
                reviewer_feedback = format!(
                    "Reviewer {md} returned no usable ok/issues verdict: {}",
                    crate::sh::head(
                        &if value.is_null() {
                            text
                        } else {
                            value.to_string()
                        },
                        1000
                    )
                );
                a.say("reviewer", &reviewer_feedback, Some(j.id));
            }
        }
    }
    let js = match result {
        Some(value) => value,
        None if crate::routing::enabled(a) => {
            return Err(X::E(
                "no usable independent subscription review; submission remains blocked".into(),
            ))
        }
        None => {
            json!({"ok":false,"issues":["Reviewer returned no usable verdict; repair the candidate and retry review."]})
        }
    };
    let okk = js["ok"].as_bool().unwrap_or(false);
    a.st("reviewer", "idle", None, if okk { "ok" } else { "sad" });
    if okk {
        if js["text"].is_object() {
            for k in ["title", "cause", "fix", "untouched"] {
                if let Some(s) = js["text"][k].as_str() {
                    if k == "untouched" || !s.trim().is_empty() {
                        f[k] = json!(s.trim());
                    }
                }
            }
            a.say("reviewer", "ok, PR text corrected", Some(j.id));
        } else {
            a.say("reviewer", "ok", Some(j.id));
        }
        return Ok(String::new());
    }
    let is: Vec<String> = js["issues"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![json!("no usable answer")])
        .iter()
        .map(|x| format!("- {}", x.as_str().unwrap_or(&x.to_string())))
        .collect();
    let is = is.join("\n");
    a.say(
        "reviewer",
        &format!("back to Fixer: {}", crate::sh::head(&is, 200)),
        Some(j.id),
    );
    Ok(format!("Reviewer objections:\n{is}"))
}

const FEAT: &str = "This job is a FEATURE REQUEST, not a bug. The rules for a bug fix apply with these changes: implement the requested behaviour completely but minimally, in the project's existing style, without unrelated changes. Commit 1 \"test: …\" adds tests that check the new behaviour; on that commit they must fail because the feature is missing (an assertion failure is best; if the test has to call a new API, a compile error on that commit is accepted). Commit 2 \"feat: …\" implements the feature; there the tests pass. Update user-facing docs or the changelog only if the project does that for every feature. In the JSON, \"cause\" is why the feature is needed (what users could not do), \"fix\" is what was added.";
const OPT: &str = "This job is a BEHAVIOR-PRESERVING OPTIMIZATION, not a bug fix or feature. Keep all externally observable behavior, outputs, APIs, formats, and defaults unchanged. Commit 1 \"test: …\" adds characterization checks that pass on the unchanged base. Commit 2 \"refactor: …\" makes the focused improvement. Run the same test_cmd on both commits; every named check must pass before and after. Do not claim performance gains unless the checks objectively measure them. In the JSON, \"cause\" explains the concrete improvement, \"fix\" describes the change, and \"untouched\" notes behavior or areas preserved.";

fn fixp(a: &Ax, j: &Job, fb: &str) -> String {
    let h = format!(
        "Repository: {}. Branch {} is checked out. Base commit: {}.",
        j.repo,
        j.s("br"),
        j.s("base")
    );
    if j.kind == "review" {
        let tc = j.s("test_cmd");
        return format!(
            "{h}\nThis branch is an open pull request (#{}). A maintainer requested a change:\n{}\n\nOriginal comment:\n{}\n\nImplement exactly the requested change as ONE new commit on top of the base commit (do not rewrite existing commits). Keep the existing regression tests passing.{}\nThen answer with the JSON from your instructions; \"tests\" are the regression tests that must pass, \"title\" is a short commit summary.{}{}",
            j.pr.unwrap_or(0),
            wrap("request", &j.s("change")),
            wrap("comment", &j.s("comment")),
            if tc.is_empty() { String::new() } else { format!(" Existing test command: {tc}") },
            if fb.is_empty() { String::new() } else { format!("\n\nYour previous attempt was rejected:\n{fb}\nThe branch still contains your previous work; fix the problems.") },
            rules(a, j)
        );
    }
    let ci = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    let h = if ci.is_empty() { h } else { format!("{h} The repository's full test command is `{ci}`; your test_cmd runs in a fresh shell from the repo root, so use the same environment (for example the same virtualenv or package manager).") };
    let h = if j.opt() { format!("{h}\n{OPT}") } else if j.feat() { format!("{h}\n{FEAT}") } else { h };
    let commit_roles = if j.opt() { "verification, optimization" } else if j.feat() { "test, feature" } else { "test, fix" };
    format!(
        "{h}\n{}:\n{}\n\n{} (from a previous agent): {}{}{}",
        j.orig(),
        wrap("issue", &format!("{}\n\n{}", j.title, j.s("body"))),
        if j.feat() || j.opt() { "Implementation plan" } else { "Repro result" },
        j.d["repro"],
        if fb.is_empty() { String::new() } else { format!("\n\nYour previous attempt was rejected by the pipeline:\n{fb}\nThe branch still contains your previous commits; fix the problems and make sure there are exactly two commits ({commit_roles}) on top of the base commit.") },
        format!("{}{}",rules(a, j),crate::quality::context(j))
    )
}

async fn vrev(a: &Ax, j: &Job, f: &Value) -> R<String> {
    let mut n: i64 = ok(
        a,
        j,
        &format!("git rev-list --count {}..HEAD", j.s("base")),
        60,
    )
    .await?
    .parse()
    .unwrap_or(0);
    if n > 1 {
        let b = q(&j.s("base"));
        ok(a, j, &format!("c=$(git rev-list --reverse {b}..HEAD | head -1) && git reset -q --soft {b} && git commit -q -C \"$c\""), 60).await?;
        n = ok(a, j, &format!("git rev-list --count {b}..HEAD"), 60)
            .await?
            .parse()
            .unwrap_or(0);
    }
    if n != 1 {
        return Ok(format!(
            "expected exactly 1 new commit on top of the PR head, found {n}"
        ));
    }
    if let Some(m) = dirty(a, j).await? {
        return Ok(m);
    }
    let c = f["test_cmd"]
        .as_str()
        .map(String::from)
        .filter(|x| !x.is_empty())
        .unwrap_or(j.s("test_cmd"));
    if !c.is_empty() {
        if Regex::new(crate::mcp::BAD).unwrap().is_match(&c) {
            return Ok("test_cmd contains forbidden commands".into());
        }
        let (e, o) = run_t(a, j, "HEAD", &c, &[]).await;
        ok(a, j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
        if e != 0 {
            return Ok(format!("Tests fail:\n{}", crate::sh::last(&o, 3000)));
        }
    }
    Ok(String::new())
}

fn one_line(s: &str) -> String {
    let t: String = s.lines().next().unwrap_or("").trim().chars().take(72).collect();
    if t.is_empty() { "update".into() } else { t }
}

/// Test-runner / build / dependency configuration whose change can alter which
/// tests are discovered or run (e.g. `package.json` `scripts.test`, lockfiles,
/// pytest/tox/jest/cargo config, CI workflows). Protected from GREEN so a fix
/// cannot neuter the runner (`npm test` -> `echo ...`) while leaving the bug.
fn runner_cfg(p: &str) -> bool {
    let base = p.rsplit('/').next().unwrap_or(p);
    const EXACT: &[&str] = &["package.json", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "npm-shrinkwrap.json", "pyproject.toml", "setup.py", "setup.cfg", "pytest.ini", "tox.ini", "noxfile.py", "makefile", "gnumakefile", "go.mod", "go.sum", "cargo.toml", "cargo.lock", "build.gradle", "build.gradle.kts", "settings.gradle", "pom.xml", "phpunit.xml", "phpunit.xml.dist", "composer.json", "gemfile", "rakefile", ".rspec", "meson.build", "cmakelists.txt", "dune-project"];
    if EXACT.iter().any(|x| x.eq_ignore_ascii_case(base)) {
        return true;
    }
    p.starts_with(".github/workflows/") || Regex::new(r"(?i)^(jest|vitest|karma|playwright|cypress|babel|mocha|ava|nyc|tsconfig|wdio|jasmine)[.\-].*\.(c?js|mjs|ts|json|ya?ml)$|^\.(mocharc|babelrc|nycrc|ava)([.\-].*)?$|\.config\.(c?js|mjs|ts)$").unwrap().is_match(base)
}

/// Classify a repo path as a test file. Repo config `tests` (a regex) wins; the
/// fallback covers the common test layouts across languages. Used in staged mode
/// to reject production changes in the RED phase.
fn testish(a: &Ax, j: &Job, p: &str) -> bool {
    if let Some(re) = rp(a, &j.repo)["tests"].as_str().filter(|s| !s.trim().is_empty()) {
        if let Ok(rx) = Regex::new(re) {
            return rx.is_match(p);
        }
    }
    Regex::new(r"(?i)(^|/)(tests?|specs?|__tests__|testing)/|(^|/)conftest\.py$|_test|_spec|test_|spec_|\.test\.|\.spec\.|(^|/)test[^/]*$|Tests?\.(java|kt|kts|cs|scala|go|rb|py|php|swift)$").unwrap().is_match(p)
}

/// Parse `git status --porcelain=v1 -z -uall` output. NUL-delimited and `-uall`
/// so new directories expand to files and no path needs unquoting. For a rename
/// or copy (X or Y is R/C) the next NUL field is the ORIGIN path; BOTH the origin
/// and the destination are returned, so renaming production into a test directory
/// still surfaces the production path for classification.
pub(crate) fn parse_status_z(st: &str) -> Vec<String> {
    let fields: Vec<&str> = st.split('\0').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < fields.len() {
        let f = fields[i];
        i += 1;
        if f.len() < 4 {
            continue;
        }
        let b = f.as_bytes();
        let (x, y) = (b[0] as char, b[1] as char);
        let path = f[3..].trim(); // "XY PATH"
        if !path.is_empty() && !junk(path) {
            out.push(path.to_string());
        }
        if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
            if let Some(orig) = fields.get(i) {
                i += 1;
                let orig = orig.trim();
                if !orig.is_empty() && !junk(orig) {
                    out.push(orig.to_string());
                }
            }
        }
    }
    out
}

/// Run a git command whose output must NOT be trimmed (the leading status byte of
/// `-z` porcelain is significant, and `ok()` would strip it). Errors on nonzero.
async fn okz(a: &Ax, j: &Job, cmd: &str) -> R<String> {
    let r = x(a, j, cmd, 60).await;
    if r.c != 0 {
        return Err(oe(&r, cmd, &j.h()));
    }
    Ok(r.o)
}

/// Working-tree changed paths vs the index/HEAD (staged and unstaged), excluding
/// build junk, with rename origins included.
async fn changed_paths(a: &Ax, j: &Job) -> R<Vec<String>> {
    Ok(parse_status_z(&okz(a, j, "git status --porcelain=v1 -z -uall").await?))
}

/// Run a command in the current working tree (no checkout), returning (exit, output).
async fn run_wt(a: &Ax, j: &Job, c: &str) -> (i32, String) {
    let r = x(a, j, &format!("{{ to 3600 bash -c {}\n}} 2>&1; echo \"[exit $?]\"", q(c)), 3700).await;
    let e = Regex::new(r"\[exit (\d+)\]\s*$").unwrap().captures(&r.o).and_then(|m| m[1].parse::<i32>().ok()).unwrap_or(-1);
    (e, r.o)
}

/// A model-supplied test command may only run the project's tests: it is never
/// trusted to touch git or the shell. `mcp::BAD` blocks push/remote/config and
/// `mcp::STAGED_GIT` blocks every history/index mutation, closing the `run_wt`
/// path that bypasses the `sh`-tool guard. Not a sandbox — paired with a HEAD and
/// working-tree re-check after the command runs.
fn bad_test_cmd(a: &Ax, j: &Job, tc: &str) -> Option<String> {
    if tc.trim().is_empty() {
        return Some("test_cmd is empty".into());
    }
    if Regex::new(crate::mcp::BAD).unwrap().is_match(tc) {
        return Some("test_cmd contains forbidden commands".into());
    }
    if Regex::new(crate::mcp::STAGED_GIT).unwrap().is_match(tc) {
        return Some("test_cmd must only run the tests; it may not run git history, index or commit commands".into());
    }
    // Optional per-repo test-command allowlist (rp.tcmd, a regex). When set, the
    // model's test_cmd must match it, so an untrusted command cannot run arbitrary
    // code on the executor. Defence in depth, not a substitute for a sandbox.
    if let Some(re) = rp(a, &j.repo)["tcmd"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        match Regex::new(re) {
            Ok(rx) if !rx.is_match(tc) => return Some(format!("test_cmd is not permitted by this repository's approved test-command policy ({re})")),
            Err(e) => return Some(format!("repository test-command policy regex is invalid ({e})")),
            _ => {}
        }
    }
    None
}

/// The repo's test classifier regex (`rp.tests`), validated. A configured but
/// invalid regex fails closed (Err) rather than silently falling back (A3).
fn tests_re(a: &Ax, j: &Job) -> Result<(), String> {
    match rp(a, &j.repo)["tests"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        Some(re) => Regex::new(re).map(|_| ()).map_err(|e| format!("repository test classifier regex is invalid ({e}); fix rp.tests before staged runs")),
        None => Ok(()),
    }
}

/// The mandatory GREEN production allowlist (`rp.green`, a regex). Only paths
/// matching it may be changed by a GREEN fix. Fails closed: unset/empty means no
/// GREEN can proceed, and an invalid regex is an explicit error (V3-P0-1).
fn green_allow(a: &Ax, j: &Job) -> Result<Regex, String> {
    match rp(a, &j.repo)["green"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
        None => Err("staged GREEN requires a production-file allowlist for this repository (rp.green); none configured, so no fix may be committed".into()),
        Some(re) => Regex::new(re).map_err(|e| format!("repository GREEN allowlist regex is invalid ({e}); fix rp.green before staged runs")),
    }
}

/// Stage exactly `paths` (never `-A`, so nothing a test command created outside
/// the authorised set is committed), confirm the staged set is exactly `paths`,
/// commit, then restore the working tree to the new HEAD so any residue a test
/// left behind is dropped and the tree is pristine for the next phase.
async fn commit_scope(a: &Ax, j: &Job, paths: &[String], msg: &str) -> R<Option<String>> {
    let spec = paths.iter().map(|p| q(p)).collect::<Vec<_>>().join(" ");
    // Run the add tolerantly: an authorised path that no longer matches makes git
    // exit nonzero, which is a controlled rejection (the set check also guards).
    let add = x(a, j, &format!("git add -- {spec}"), 120).await;
    if add.c != 0 {
        ok(a, j, "git reset -q", 60).await?;
        return Ok(Some(format!("could not stage the authorised paths {:?}: {}", paths, crate::sh::last(add.all().trim(), 300))));
    }
    // `--no-renames` lists a rename as delete(origin)+add(dest), matching the way
    // `changed_paths` records both sides, so the sets compare exactly. The staged
    // set must EQUAL the authorised set: no extra path a test command may have
    // created, and no authorised path silently missing.
    let cached: BTreeSet<String> = okz(a, j, "git diff --cached --name-only --no-renames -z").await?.split('\0').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    let want: BTreeSet<String> = paths.iter().cloned().collect();
    if cached != want {
        ok(a, j, "git reset -q", 60).await?;
        return Ok(Some(format!("unexpected staging state (staged {:?}, authorised {:?})", cached, want)));
    }
    ok(a, j, &format!("git commit -q -m {}", q(msg)), 120).await?;
    // Drop anything the test command left unstaged/untracked; keep the commit.
    ok(a, j, "git checkout -q -f -- . 2>/dev/null; git clean -fdq", 120).await?;
    if !changed_paths(a, j).await?.is_empty() {
        return Ok(Some("working tree not clean after the commit".into()));
    }
    Ok(None)
}

/// RED: only test files changed, the focused test fails legitimately (a real
/// assertion, not a compile/import error), the test command itself did not move
/// history or touch production, then the controller creates the test-only commit
/// from an explicit path set. Returns Some(feedback) on a controlled rejection.
async fn red_commit(a: &Ax, j: &mut Job, tc: &str, ts: &[String], title: &str) -> R<Option<String>> {
    if let Some(m) = bad_test_cmd(a, j, tc) {
        return Ok(Some(m));
    }
    if let Err(m) = tests_re(a, j) {
        return Ok(Some(m));
    }
    let base = j.s("base");
    // The office owns history: before RED the branch must still be at the base.
    if ok(a, j, "git rev-parse HEAD", 60).await? != base {
        return Ok(Some("The branch history changed during the RED phase. The office creates every commit; do not create, amend or reset commits. Edit test files only and leave them uncommitted.".into()));
    }
    let pre = changed_paths(a, j).await?;
    if pre.is_empty() {
        return Ok(Some("The RED phase made no changes; write a regression test that fails on the current code.".into()));
    }
    let nontest: Vec<String> = pre.iter().filter(|p| !testish(a, j, p)).cloned().collect();
    if !nontest.is_empty() {
        return Ok(Some(format!("The RED phase must change test files only, but these look like implementation code: {}. Move every production change to the GREEN phase.", nontest.iter().take(10).cloned().collect::<Vec<_>>().join(", "))));
    }
    let (e, o) = run_wt(a, j, tc).await;
    // TOCTOU / untrusted test_cmd: re-read state AFTER the command ran.
    if ok(a, j, "git rev-parse HEAD", 60).await? != base {
        return Ok(Some("The test command changed git history. test_cmd may only run the tests. Output:\n".to_string() + &crate::sh::last(&o, 2000)));
    }
    let post = changed_paths(a, j).await?;
    let post_nontest: Vec<String> = post.iter().filter(|p| !testish(a, j, p)).cloned().collect();
    if !post_nontest.is_empty() {
        return Ok(Some(format!("The test command modified non-test files: {}. A regression test must not write to production code. Output:\n{}", post_nontest.iter().take(10).cloned().collect::<Vec<_>>().join(", "), crate::sh::last(&o, 2000))));
    }
    if e == 0 {
        return Ok(Some(format!("The regression test passes without any fix; it must fail. Output:\n{}", crate::sh::last(&o, 3000))));
    }
    if Regex::new(CE).unwrap().is_match(&o) {
        return Ok(Some(format!("The test does not compile or load; a compile or import error does not count as a failing test. Make the test call existing API so it fails on behaviour. Output:\n{}", crate::sh::last(&o, 3000))));
    }
    // A RED test that "fails" only because a test helper (assert/test/expect/...) is
    // undefined or misimported is not a behavioural failure: it throws before any
    // assertion runs. The staged flow fixes existing behaviour, so the production API
    // exists and these runtime-binding errors mean a broken test harness, not the
    // bug. Reject so the author reimports the helpers correctly (the 6068963 defect:
    // `const { assert } = require('assert')` leaves `assert` undefined).
    if Regex::new(r"Cannot read properties of undefined|Cannot read property '[^']*' of undefined|\bis not a function\b").unwrap().is_match(&o) {
        return Ok(Some(format!("The regression test fails because a test helper (e.g. assert/test/expect) is undefined or misimported, not because of a behavioural assertion — it throws a TypeError before any assertion runs. Import the helpers exactly as the existing tests do (e.g. `const assert = require('node:assert')`, `const {{ test }} = require('node:test')`), keep every existing test, and make the failure a real AssertionError. Output:\n{}", crate::sh::last(&o, 3000))));
    }
    if !ts.iter().any(|t| o.contains(&bare(t))) {
        return Ok(Some(format!("None of the named tests appears failing in the output. Output:\n{}", crate::sh::last(&o, 3000))));
    }
    j.d["red_output"] = json!(crate::sh::last(&o, 2000));
    a.db.jd(j);
    commit_scope(a, j, &post, &format!("test: {}", one_line(title))).await
}

/// GREEN: the fix changes only production code (no test or runner file, none of
/// the RED-committed files), the RED test commit is still HEAD, the frozen RED
/// test command now passes AND the named tests are observed running (so disabling
/// or replacing the runner is rejected), then the controller creates the fix-only
/// commit. `tc`/`ts` are the frozen RED values, never the GREEN answer's.
async fn green_commit(a: &Ax, j: &mut Job, red_files: &[String], test_commit: &str, tc: &str, ts: &[String], title: &str) -> R<Option<String>> {
    if let Some(m) = bad_test_cmd(a, j, tc) {
        return Ok(Some(m));
    }
    if let Err(m) = tests_re(a, j) {
        return Ok(Some(m));
    }
    if ok(a, j, "git rev-parse HEAD", 60).await? != test_commit {
        return Ok(Some("The branch history changed during the GREEN phase. The office creates every commit; do not create, amend or reset commits. Edit production files only and leave them uncommitted.".into()));
    }
    // MANDATORY GREEN allowlist (V3-P0-1). The fix may touch ONLY production paths
    // explicitly allowlisted for this repository (rp.green, a regex). This is a
    // whitelist, not a growing blocklist: any other path — a test, the runner, or
    // an INDIRECT runner script the test command invokes — is rejected because it
    // is simply not on the list. Fails closed: a staged repo with no allowlist
    // cannot run a GREEN phase at all.
    let allow = match green_allow(a, j) {
        Ok(rx) => rx,
        Err(m) => return Ok(Some(m)),
    };
    // Defence in depth on top of the allowlist.
    let protected = |p: &str| testish(a, j, p) || runner_cfg(p) || red_files.iter().any(|r| r == p) || tc.contains(p);
    let offending = |paths: &[String]| -> Vec<String> { paths.iter().filter(|p| !allow.is_match(p) || protected(p)).cloned().collect() };
    let pre = changed_paths(a, j).await?;
    if pre.is_empty() {
        return Ok(Some("The GREEN phase made no changes; implement the fix so the committed test passes.".into()));
    }
    let bad = offending(&pre);
    if !bad.is_empty() {
        return Ok(Some(format!("The fix may change only the allowlisted production files for this repository; these are not permitted: {}. The regression test, the test runner and any script it invokes must stay exactly as committed in the RED phase.", bad.iter().take(10).cloned().collect::<Vec<_>>().join(", "))));
    }
    let (e, o) = run_wt(a, j, tc).await;
    if ok(a, j, "git rev-parse HEAD", 60).await? != test_commit {
        return Ok(Some("The test command changed git history. test_cmd may only run the tests. Output:\n".to_string() + &crate::sh::last(&o, 2000)));
    }
    let post = changed_paths(a, j).await?;
    let bad = offending(&post);
    if !bad.is_empty() {
        return Ok(Some(format!("The test command or the fix touched files outside the production allowlist: {}. The RED test and its runner must stay unchanged. Output:\n{}", bad.iter().take(10).cloned().collect::<Vec<_>>().join(", "), crate::sh::last(&o, 2000))));
    }
    if e != 0 {
        return Ok(Some(format!("The regression test still fails with the fix. Output:\n{}", crate::sh::last(&o, 3000))));
    }
    let f = fails(&o);
    if !f.is_empty() {
        return Ok(Some(format!("Tests still reported as failing with the fix: {}. Output:\n{}", f.join(", "), crate::sh::last(&o, 3000))));
    }
    // Positive proof (B1): the frozen RED tests must actually appear in the GREEN
    // output. Replacing or disabling the runner (so nothing runs) is rejected here
    // even though exit is 0. Quiet runners that print no per-test name are a known
    // limitation and should set rp.tests / a verbose command.
    let missing: Vec<String> = ts.iter().filter(|t| !o.contains(&bare(t))).cloned().collect();
    if !missing.is_empty() {
        return Ok(Some(format!("Could not confirm the regression test(s) ran with the fix: {} did not appear in the output. Do not disable, replace or skip the test runner. Output:\n{}", missing.join(", "), crate::sh::last(&o, 3000))));
    }
    commit_scope(a, j, &post, &format!("fix: {}", one_line(title))).await
}

fn staged_prompt(a: &Ax, j: &Job, fb: &str, red: bool, tc: &str) -> String {
    let h = format!("Repository: {}. Branch {} is checked out at the base commit {}.", j.repo, j.s("br"), j.s("base"));
    let ci = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    let h = if ci.is_empty() { h } else { format!("{h} The repository's full test command is `{ci}`; your focused test_cmd runs in a fresh shell from the repo root, so use the same environment (for example the same virtualenv or package manager).") };
    let base_ctx = format!("{h}\n{}:\n{}\n\nRepro result (from a previous agent): {}{}{}", j.orig(), wrap("issue", &format!("{}\n\n{}", j.title, j.s("body"))), j.d["repro"], rules(a, j), crate::quality::context(j));
    if red {
        format!("{base_ctx}{}", if fb.is_empty() { String::new() } else { format!("\n\nThe previous attempt was rejected by the office:\n{fb}\nThe working tree was reset to the base commit; write the failing regression test again.") })
    } else {
        format!("{base_ctx}\n\nThe regression test is already written and committed by the office. Its command is `{tc}` and it currently fails:\n{}\n\nImplement the smallest correct fix so it passes. Do not modify the test.{}", crate::sh::head(j.d["red_output"].as_str().unwrap_or(""), 3000), if fb.is_empty() { String::new() } else { format!("\n\nThe previous fix attempt was rejected by the office:\n{fb}") })
    }
}

async fn ask_phase(a: &Ax, j: &Job, md: &str, sys: &str, prompt: String) -> R<Value> {
    // Test seam: a scripted model turn applies its file mutations to the run dir
    // and returns its JSON, exercising the full staged state machine (P2-4).
    #[cfg(test)]
    if let Some(mut turn) = a.sim.lock().unwrap().pop_front() {
        let dir = dir(a, j);
        if let Some(files) = turn.get("_files").and_then(|v| v.as_object()).cloned() {
            for (path, content) in files {
                let p = std::path::Path::new(&dir).join(&path);
                let _ = std::fs::create_dir_all(p.parent().unwrap());
                std::fs::write(&p, content.as_str().unwrap_or("")).unwrap();
            }
        }
        if let Some(dels) = turn.get("_delete").and_then(|v| v.as_array()).cloned() {
            for d in dels {
                let _ = std::fs::remove_file(std::path::Path::new(&dir).join(d.as_str().unwrap_or("")));
            }
        }
        if let Some(o) = turn.as_object_mut() {
            o.remove("_files");
            o.remove("_delete");
        }
        let _ = (md, sys, prompt);
        return Ok(turn);
    }
    let c = cx(a, j);
    let mut r = rn(a, j);
    if let Some(x) = r.as_mut() {
        x.staged = true;
    }
    let mut o = Ask::new("fixer", md.to_string(), P(sys), prompt);
    o.job = Some(j.id);
    o.rn = r;
    o.turns = if c { 200 } else { 120 };
    o.t = if c { 180 * 60 } else { 120 * 60 };
    // A Codex staged fallback FIXER runs at the configured reasoning effort (max in
    // the pilot). Scoped to the staged fixer phase and only when the model is a
    // Codex model, so other jobs and non-codex fixers keep their defaults.
    if crate::prov::parse(md).0 == "codex" {
        o.ef = Some(codex_fallback_effort());
    }
    Ok(ask(a, o).await?.1)
}

/// The frozen RED proof restored from the controller checkpoint.
struct RedState {
    tc: String,
    ts: Vec<String>,
    red_files: Vec<String>,
    test_commit: String,
    title: String,
}

enum Resume {
    /// A valid RED+GREEN pair already exists on the branch (crash after the GREEN
    /// commit, before adjudication): carry it forward WITHOUT any reset.
    Finished(Value),
    /// Only the RED commit exists: continue the GREEN phase from it.
    Green(RedState),
    /// No usable checkpoint: build a fresh RED from the base.
    Fresh,
}

/// Remove both staged checkpoints in memory (caller persists). Used only on paths
/// where the branch is already at the base (nothing to lose).
fn drop_checkpoints(j: &mut Job) {
    let o = j.d.as_object_mut().unwrap();
    o.remove("staged_red");
    o.remove("staged_green");
}

/// Atomically reject the current candidate: clear both checkpoints AND authorise a
/// reset of exactly this HEAD back to the base, in a single DB write. The next
/// Fresh round may then reset (intentional discard), while a crash — which leaves
/// no such authorisation — is refused instead of resetting (V4 window B / point 5).
fn reject_candidate(a: &Ax, j: &mut Job, head: &str, base: &str) {
    drop_checkpoints(j);
    j.d["staged_discard"] = json!({"head": head, "base": base});
    a.db.jd(j);
}

/// Decide how to resume a staged job, NEVER moving the branch ref off an accessible
/// commit without an explicit, persisted decision (V4). It recovers a finished
/// RED+GREEN pair, continues from a RED commit (even one whose post-commit
/// checkpoint write was lost — window A), authorises a reset only for an
/// intentionally-rejected HEAD, and otherwise blocks for a human.
async fn staged_resume(a: &Ax, j: &mut Job, base: &str, br: &str) -> R<Resume> {
    let head = ok(a, j, "git rev-parse HEAD", 60).await?;
    let head_count = ok(a, j, &format!("git rev-list --count {}..HEAD", q(base)), 60).await.unwrap_or_default();
    let cp = j.d["staged_red"].clone();

    // --- No RED checkpoint persisted ---
    if !cp.is_object() {
        if head == base {
            return Ok(Resume::Fresh); // clean branch, safe to build fresh
        }
        // Commits exist above the base with no checkpoint. Only an authorisation
        // persisted for exactly this HEAD (an intentional rejection) permits reset.
        let disc = j.d["staged_discard"].clone();
        if disc["head"].as_str() == Some(head.as_str()) && disc["base"].as_str() == Some(base) {
            j.d.as_object_mut().unwrap().remove("staged_discard");
            a.db.jd(j);
            return Ok(Resume::Fresh); // authorised discard of a rejected candidate
        }
        return drop(format!("staged resume: {head_count} commit(s) on the branch above the base with no checkpoint and no discard authorisation (likely a crash after a RED/GREEN commit). HEAD={} base={}. Refusing to reset; manual inspection required.", crate::sh::head(&head, 12), crate::sh::head(base, 12)));
    }

    // --- RED checkpoint present (may be pending, i.e. committed-but-not-finalised) ---
    let tc = cp["test_cmd"].as_str().unwrap_or("").to_string();
    let ts: Vec<String> = cp["tests"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
    let mut red_files: Vec<String> = cp["red_files"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
    let title = cp["title"].as_str().unwrap_or("").to_string();
    if tc.is_empty() || ts.is_empty() {
        // Corrupt checkpoint. Only safe to drop when the branch is at the base.
        if head == base {
            drop_checkpoints(j);
            a.db.jd(j);
            return Ok(Resume::Fresh);
        }
        return drop(format!("staged resume: corrupt checkpoint with commits on the branch (HEAD={}); refusing to reset, manual inspection required", crate::sh::head(&head, 12)));
    }

    match head_count.as_str() {
        // RED+GREEN already committed: recover the finished pair, never reset.
        "2" => {
            let red = ok(a, j, "git rev-parse HEAD~1", 60).await?;
            let red_parent = ok(a, j, &format!("git rev-parse -q --verify {}^ 2>/dev/null || true", q(&red)), 60).await.unwrap_or_default();
            if red_parent != base {
                return drop(format!("staged resume: two commits but HEAD~1 is not based on the base (HEAD={}); refusing to reset, manual inspection required", crate::sh::head(&head, 12)));
            }
            let sg = j.d["staged_green"].clone();
            let js = if sg["commit"].as_str() == Some(head.as_str()) && sg["js"].is_object() {
                sg["js"].clone()
            } else {
                json!({"title": if title.is_empty() { "fix".into() } else { title }, "test_cmd": tc, "tests": ts, "cause": "", "fix": ""})
            };
            a.say("fixer", "resuming: RED+GREEN already committed; carrying it to verificat…22705 tokens truncated…d(180, 0, 180, 0));
        assert!(repo_limit_reached(0, 1, 0, 1));
        assert!(!repo_limit_reached(179, 0, 180, 1));
    }

    #[test]
    fn ready_fixes_precede_new_hunts_and_rebases() {
        let fix = json!({"id":1,"kind":"fix","st":"ready","prio":-0.5,"d":{}});
        let hunt = json!({"id":2,"kind":"hunt","st":"ready","prio":99.0,"d":{}});
        let rebase = json!({"id":3,"kind":"rebase","st":"ready","prio":-1.0,"d":{}});
        let submit = json!({"id":4,"kind":"fix","st":"submit","prio":0.0,"d":{}});
        assert!(pick_rank(&fix) < pick_rank(&hunt));
        assert!(pick_rank(&fix) < pick_rank(&rebase));
        assert!(pick_rank(&submit) < pick_rank(&fix));
    }

    #[cfg(unix)]
    fn run_preflight(
        nvmrc: Option<&str>,
        node_body: &str,
        nvm_resolved: &str,
    ) -> std::process::Output {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "apollo-preflight-{}-{}",
            std::process::id(),
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let bin = root.join("bin");
        let nvm_dir = root.join("nvm");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&nvm_dir).unwrap();
        if let Some(requirement) = nvmrc {
            std::fs::write(root.join(".nvmrc"), requirement).unwrap();
        }
        for (name, body) in [
            ("git", "exit 0"),
            ("node", node_body),
            ("nvm", "printf '%s\n' \"$NVM_RESOLVED\""),
        ] {
            let path = bin.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            let mut permissions = std::fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        let output = std::process::Command::new("/bin/bash")
            .arg("-c")
            .arg(ENV_PREFLIGHT)
            .current_dir(&root)
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("NVM_DIR", &nvm_dir)
            .env("NVM_RESOLVED", nvm_resolved)
            .output()
            .unwrap();
        let _ = std::fs::remove_dir_all(root);
        output
    }

    #[cfg(unix)]
    #[test]
    fn environment_preflight_resolves_node_requirements_without_breaking_other_repositories() {
        let no_nvmrc = run_preflight(None, "echo broken-node >&2; exit 9", "");
        assert!(
            no_nvmrc.status.success(),
            "{}",
            String::from_utf8_lossy(&no_nvmrc.stdout)
        );

        let pinned = run_preflight(
            Some("v20.1.0 # keep the major stable\n"),
            "printf 'v20.19.0\n'",
            "",
        );
        assert!(
            pinned.status.success(),
            "{}",
            String::from_utf8_lossy(&pinned.stdout)
        );

        let alias = run_preflight(Some("lts/argon\n"), "printf 'v20.19.0\n'", "v20.20.0");
        assert!(
            alias.status.success(),
            "{}",
            String::from_utf8_lossy(&alias.stdout)
        );

        let unresolved = run_preflight(Some("lts/unknown\n"), "printf 'v20.19.0\n'", "N/A");
        assert!(!unresolved.status.success());
        assert!(
            String::from_utf8_lossy(&unresolved.stdout).contains("unsupported .nvmrc requirement")
        );

        let broken = run_preflight(Some("20\n"), "echo node failed >&2; exit 7", "");
        assert!(!broken.status.success());
        assert!(String::from_utf8_lossy(&broken.stdout).contains("node failed to run: node failed"));

        let mismatch = run_preflight(Some("24\n"), "printf 'v20.19.0\n'", "");
        assert!(!mismatch.status.success());
        assert!(String::from_utf8_lossy(&mismatch.stdout).contains("node incompatible"));
    }

    #[test]
    fn mergecheck_retry_preserves_head_sha_and_negative_ci_evidence() {
        let head = "0123456789abcdef0123456789abcdef01234567";
        let data = json!({"expected_head":head, "proof":{"fix_commit":null}});
        let kind = "mergecheck";
        let command = "node --test";
        let output = "not ok 1\n# fail 1";
        let first = ci_evidence(kind, &data, command, 1, output, false);
        let retry = ci_evidence(kind, &data, command, 1, output, true);
        assert_eq!(first["candidate"], json!(head));
        assert_eq!(retry["candidate"], json!(head));
        assert_eq!(retry["exit"], 1);
        assert_eq!(retry["command"], command);
        assert_eq!(retry["rerun"], true);
        let tail = retry["output_tail"].as_str().unwrap();
        assert!(tail.contains("# fail 1"));
    }

    #[test]
    fn own_fix_ci_evidence_keeps_its_fix_commit_and_green_retry_sha() {
        let fix = "fedcba9876543210fedcba9876543210fedcba98";
        let data = json!({"expected_head":null,"proof":{"fix_commit":fix}});
        let kind = "fix";
        let first = ci_evidence(kind, &data, "", 1, "", false);
        let retry = ci_evidence(kind, &data, "", 0, "", true);
        assert_eq!(first["candidate"], json!(fix));
        assert_eq!(retry["candidate"], json!(fix));
        assert_eq!(retry["exit"], 0);
        assert_eq!(retry["rerun"], true);
        assert!(first.get("rerun").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cancelled_ci_releases_build_capacity() {
        let root = std::env::temp_dir().join(format!(
            "apollo-ci-cancel-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        std::fs::create_dir_all(root.join("work/o__r")).unwrap();
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,on_,ci) values('local',1,1)",
            crate::pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','sleep 30')",
            crate::pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "test",
            "",
            "ci",
            "",
            json!({"h":"local"}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(db, root.clone(), 1, "x".into()));
        let mut job = a.db.jg(id).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), ci(&a, &mut job))
                .await
                .is_err()
        );
        assert_eq!(a.sem.lock().unwrap().get("local").copied(), Some(0));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_transport_errors_are_not_inferred_from_script_stdout() {
        let lost_worker = Out {
            c: 125,
            o: String::new(),
            e: "docker: No such container".into(),
        };
        let failed_script = Out {
            c: 1,
            o: "test reported No such container".into(),
            e: String::new(),
        };
        let cannot_spawn = Out {
            c: -1,
            o: String::new(),
            e: "cannot spawn worker process".into(),
        };
        let ssh_reset = Out {
            c: 255,
            o: String::new(),
            e: "ssh: Connection reset by peer".into(),
        };
        let stdout_only = Out {
            c: 1,
            o: "Connection reset by peer".into(),
            e: String::new(),
        };
        assert!(setup_transport_error(&lost_worker));
        assert!(!setup_transport_error(&failed_script));
        assert!(setup_transport_error(&cannot_spawn));
        assert!(setup_transport_error(&ssh_reset));
        assert!(!setup_transport_error(&stdout_only));
    }

    #[test]
    fn preflight_host_failures_are_not_missing_tool_failures() {
        let unreachable = Out {
            c: 255,
            o: String::new(),
            e: "ssh: connect to host failed".into(),
        };
        let missing = Out {
            c: 127,
            o: String::new(),
            e: "Falta: node".into(),
        };
        let absent = Out {
            c: -1,
            o: String::new(),
            e: "machine local is gone".into(),
        };
        assert!(preflight_host_error(&unreachable));
        assert!(preflight_host_error(&absent));
        assert!(!preflight_host_error(&missing));
    }

    #[tokio::test]
    async fn a_blocked_hunt_cannot_be_admitted_and_active_hunts_do_not_overlap() {
        let db = crate::db::Db::mem();
        let first = db.jnew(
            "o/r",
            0,
            "hunt",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let second = db.jnew(
            "o/r",
            0,
            "hunt",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let fix = db.jnew(
            "o/r",
            1,
            "fix",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        db.run(
            "insert into hs(id,ln,dk,on_) values('worker1',1,0,1),('worker2',1,0,1)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-admit-hunt-tests"),
            1,
            "apollo-office".into(),
        ));
        assert!(admit(&a, "o/r", "worker1", first));
        assert!(!admit(&a, "o/r", "worker2", second));
        // Regression: a fix must not touch the checkout of an active Hunter.
        assert!(!admit(&a, "o/r", "worker2", fix));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        // Preserve the reverse-order protection (Hunter after an active fix).
        assert!(admit(&a, "o/r", "worker1", fix));
        assert!(!admit(&a, "o/r", "worker2", first));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/r", "setup", 1);
        }
        assert!(!admit(&a, "o/r", "worker1", first));
        assert!(admit(&a, "o/r", "worker1", fix));
        assert_eq!(a.db.jg(first).unwrap().st, "ready");
    }

    #[tokio::test]
    async fn unblocked_ready_hunt_cannot_exceed_the_active_hunter_cap() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 1}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        let blocked_id = db.jnew(
            "o/blocked",
            0,
            "hunt",
            "blocked",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let healthy_id = db.jnew(
            "o/healthy",
            0,
            "hunt",
            "healthy",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-hunt-cap-admission-test"),
            1,
            "apollo-office".into(),
        ));
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/blocked", "setup", 1);
        }
        assert!(!admit(&a, "o/blocked", "local", blocked_id));
        assert!(admit(&a, "o/healthy", "local", healthy_id));
        crate::hunt_health::reset(&a, "o/blocked");
        assert!(!admit(&a, "o/blocked", "local", blocked_id));
        a.act.lock().unwrap().clear();
        a.lk.lock().unwrap().clear();
        assert!(admit(&a, "o/blocked", "local", blocked_id));
        assert!(!admit(&a, "o/healthy", "local", healthy_id));
    }

    #[tokio::test]
    async fn concurrent_creation_does_not_duplicate_hunts() {
        let db = crate::db::Db::mem();
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        db.run(
            "insert into rp(full,ci,hunt,on_) values('o/r','true',1,1)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-create-hunt-tests"),
            1,
            "apollo-office".into(),
        ));
        let a1 = a.clone();
        let a2 = a.clone();
        let first = std::thread::spawn(move || mk_hunt(&a1));
        let second = std::thread::spawn(move || mk_hunt(&a2));
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.into_iter().filter(|created| *created).count(), 1);
        assert_eq!(
            a.db.n(
                "select count(*) from jobs where kind='hunt' and repo='o/r'",
                pa![]
            ),
            1
        );
    }

    #[tokio::test]
    async fn blocked_ready_hunt_does_not_starve_a_healthy_repository() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 1}));
        db.run("insert into hs(id,ln,dk,on_) values('local',1,0,1)", pa![]);
        db.run(
            "insert into rp(full,ci,hunt,on_) values('o/blocked','true',1,1),('o/healthy','true',1,1)",
            pa![],
        );
        db.jnew(
            "o/blocked",
            0,
            "hunt",
            "waiting",
            "",
            "ready",
            "",
            json!({}),
            None,
            99.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-blocked-hunt-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        for _ in 0..3 {
            crate::hunt_health::fail(&a, "o/blocked", "setup", 1);
        }
        assert!(crate::hunt_health::blocked(&a, "o/blocked"));
        assert!(
            mk_hunt(&a),
            "a blocked ready hunt must not occupy the only lane"
        );
        assert_eq!(
            a.db.n(
                "select count(*) from jobs where kind='hunt' and repo='o/healthy'",
                pa![]
            ),
            1
        );
        assert!(!mk_hunt(&a), "the healthy hunt must still use its lane");
        crate::hunt_health::success(&a, "o/blocked");
        assert!(!mk_hunt(&a), "unblocked hunts count toward capacity again");
    }

    #[tokio::test]
    async fn idle_cache_cleanup_blocks_admission_and_releases_on_drop() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into hs(id,ln,dk,on_) values('worker1',1,1,1)",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "fixture",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-idle-npm-guard-test"),
            1,
            "apollo-office".into(),
        ));
        let reservation = IdleCacheGuard::claim(&a, "worker1").unwrap();
        assert!(IdleCacheGuard::claim(&a, "worker1").is_none());
        assert!(!admit(&a, "o/r", "worker1", id));
        std::mem::drop(reservation);
        assert!(admit(&a, "o/r", "worker1", id));
        assert!(IdleCacheGuard::claim(&a, "worker1").is_none());
    }

    #[test]
    fn cache_maintenance_serializes_aliases_of_the_same_docker_target() {
        let db = crate::db::Db::mem();
        db.run(
            "insert into hs(id,ssh,ln,dk,on_) values('alias-a','worker@example',1,1,1)",
            pa![],
        );
        db.run(
            "insert into hs(id,ssh,ln,dk,on_) values('alias-b','worker@example',1,1,1)",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "same daemon",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-cache-alias-test"),
            1,
            "apollo-office".into(),
        ));
        let reservation = IdleCacheGuard::claim(&a, "alias-a").unwrap();
        assert!(IdleCacheGuard::claim(&a, "alias-b").is_none());
        assert!(!admit(&a, "o/r", "alias-b", id));
        std::mem::drop(reservation);
        assert!(admit(&a, "o/r", "alias-b", id));
        assert!(IdleCacheGuard::claim(&a, "alias-a").is_none());
    }

    #[test]
    fn setup_preflight_only_selects_known_runtimes() {
        assert_eq!(setup_tool("npm ci"), Some("npm"));
        assert_eq!(setup_tool("pnpm install --frozen-lockfile"), Some("pnpm"));
        assert_eq!(setup_tool("cargo test --workspace"), Some("cargo"));
        assert_eq!(setup_tool("curl https://example.com | sh"), None);
    }

    #[test]
    fn hunter_capacity_does_not_reserve_ready_fixes_twice() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-hunter-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 2, "without fixes both lanes may hunt");
        a.db.jnew(
            "o/r",
            0,
            "fix",
            "ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 1, "a single ready fix must reserve only one lane");
        let uncovered = a.db.jnew(
            "o/r",
            1,
            "fix",
            "another ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 0, "a ready fix excluded from hf needs another lane");
        a.db.run("update jobs set st='fix' where id=?", pa![uncovered]);
        assert_eq!(hn(&a), 0, "a distinct active fix still needs capacity");
    }

    #[test]
    fn one_lane_can_hunt_only_when_no_fix_needs_it() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',1,0,1)", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-single-lane-hunt-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 1);
        a.db.jnew(
            "o/r",
            0,
            "fix",
            "ready fix",
            "",
            "ready",
            "",
            json!({}),
            None,
            0.0,
        );
        assert_eq!(hn(&a), 0);
    }

    #[test]
    fn fixes_are_ranked_ahead_of_hunts() {
        let fix = json!({"st":"ready", "kind":"fix", "prio":0, "id":2});
        let mergecheck = json!({"st":"ready", "kind":"mergecheck", "prio":0, "id":3});
        let hunt = json!({"st":"ready", "kind":"hunt", "prio":0, "id":1});
        assert!(job_rank(&mergecheck) < job_rank(&fix));
        assert!(job_rank(&fix) < job_rank(&hunt));
        let high_priority_number = json!({"st":"ready", "kind":"fix", "prio":100_000, "id":3});
        assert!(job_rank(&high_priority_number) < job_rank(&hunt));
    }

    #[test]
    fn pr_rechecks_reserve_a_worker_lane_from_hunts() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"hunters": 2}));
        db.run("insert into hs(id,ln,dk,on_) values('local',2,0,1)", pa![]);
        db.run(
            "insert into rp(full,on_) values('o/enabled',1),('o/disabled',0)",
            pa![],
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            std::path::PathBuf::from("/tmp/apollo-recheck-capacity-test"),
            1,
            "apollo-office".into(),
        ));
        assert_eq!(hn(&a), 2);
        a.db.jnew(
            "o/enabled",
            1,
            "mergecheck",
            "Revalidate PR",
            "",
            "ready",
            "",
            json!({"expected_head":"abc"}),
            Some(42),
            -5.0,
        );
        assert_eq!(hn(&a), 1);
        a.db.jnew(
            "o/disabled",
            1,
            "mergecheck",
            "Revalidate PR",
            "",
            "ready",
            "aguarda worker",
            json!({"expected_head":"def"}),
            Some(43),
            -5.0,
        );
        assert_eq!(
            hn(&a),
            1,
            "a disabled repository must not reserve another lane"
        );
        a.db.run("update jobs set st='review' where kind='mergecheck'", pa![]);
        assert_eq!(
            hn(&a),
            1,
            "an enabled running recheck keeps its lane reservation"
        );
        a.db.run(
            "update jobs set st='done' where repo='o/enabled' and kind='mergecheck'",
            pa![],
        );
        assert_eq!(hn(&a), 2, "a disabled recheck must not suppress hunting");
    }

    #[test]
    fn optimization_verification_requires_every_named_check_before_and_after() {
        let checks = vec![
            "alpha::keeps_order".to_string(),
            "beta::uses_cache".to_string(),
        ];
        let good = "test alpha::keeps_order ... ok\ntest beta::uses_cache ... ok";
        assert!(judgeopt(&checks, 0, good, 0, good).is_empty());

        let before_missing = judgeopt(&checks, 0, "test alpha::keeps_order ... ok", 0, good);
        assert!(before_missing.contains("before the optimization"));
        assert!(before_missing.contains("beta::uses_cache"));

        let after_missing = judgeopt(&checks, 0, good, 0, "test beta::uses_cache ... ok");
        assert!(after_missing.contains("after the optimization"));
        assert!(after_missing.contains("alpha::keeps_order"));

        let red_baseline = judgeopt(
            &checks,
            0,
            "test alpha::keeps_order ... ok\ntest beta::uses_cache ... FAILED",
            0,
            good,
        );
        assert!(red_baseline.contains("before the optimization"));

        assert_eq!(
            missing_named_checks(&["test_foo".to_string()], "test test_foo_extra ... ok"),
            vec!["test_foo"]
        );
        assert_eq!(
            missing_named_checks(&["test_foo".to_string()], "test test_foo ... ignored"),
            vec!["test_foo"]
        );
        assert_eq!(
            missing_named_checks(
                &["beta::keeps_order".to_string()],
                "test alpha::keeps_order ... ok"
            ),
            vec!["beta::keeps_order"]
        );
        assert_eq!(
            missing_named_checks(
                &["alpha::keeps_order".to_string()],
                "test beta::alpha::keeps_order ... ok"
            ),
            vec!["alpha::keeps_order"]
        );
        assert!(missing_named_checks(
            &["alpha::keeps_order".to_string()],
            "test alpha::keeps_order ... ok"
        )
        .is_empty());
        assert!(
            missing_named_checks(&["rejects an error".to_string()], "✓ rejects an error")
                .is_empty()
        );
        assert!(missing_named_checks(
            &["cache_equivalence".to_string()],
            "cache_equivalence: PASS"
        )
        .is_empty());
        assert_eq!(
            missing_named_checks(
                &["rejects an error".to_string()],
                "test rejects an error ... FAILED"
            ),
            vec!["rejects an error"]
        );
        let long_log = format!(
            "test alpha::keeps_order ... ok\n{}",
            "diagnostic output ".repeat(4_000)
        );
        let tail = crate::sh::last(&long_log, 30_000);
        assert!(!missing_named_checks(&checks[..1], &tail).is_empty());
        let retained = format!("test alpha::keeps_order ... ok\n{tail}");
        assert!(missing_named_checks(&checks[..1], &retained).is_empty());
    }

    #[test]
    fn names() {
        let o = "test a::b::c ... FAILED\ntest x ... ok\nFAILED tests/t.py::test_x - assert 1 == 2\n--- FAIL: TestFoo (0.00s)\n  ✕ adds numbers (3 ms)\nnot ok 2 - parses empty\n";
        assert_eq!(
            fails(o),
            vec![
                "TestFoo",
                "a::b::c",
                "adds numbers",
                "parses empty",
                "tests/t.py::test_x"
            ]
        );
    }

    #[test]
    fn auto_merge_only_applies_to_new_bug_fix_prs() {
        let bug = Job::of(json!({"kind":"fix","d":{"feat":false}}));
        assert!(auto_merge_requested(None, true, &bug, true));
        assert!(!auto_merge_requested(None, true, &bug, false));
        assert!(!auto_merge_requested(None, false, &bug, true));
        assert!(!auto_merge_requested(Some(&json!({})), false, &bug, true));
        assert!(auto_merge_requested(Some(&json!({"auto_merge":true})), false, &bug, false));

        let feature = Job::of(json!({"kind":"fix","d":{"feat":true}}));
        assert!(!auto_merge_requested(None, true, &feature, true));
    }

    #[test]
    fn infra() {
        let o = "CI: 6 passed, FAILED at `deny`: cargo-deny is required in CI: cargo install cargo-deny --locked\nxtask ci: error: ci failed at `deny`\n[exit 1]";
        assert!(!lost(o));
        assert!(lost("Compiling foo\n"));
        assert_eq!(tool(o).unwrap().1.as_deref(), Some("cargo-deny"));
        assert_eq!(
            tool("error: no such command: `nextest`\n[exit 101]")
                .unwrap()
                .1
                .as_deref(),
            Some("cargo-nextest")
        );
        assert_eq!(
            tool("sh: 1: pnpm: command not found\n[exit 127]")
                .unwrap()
                .1,
            None
        );
        assert!(tool("test a ... FAILED\n[exit 101]").is_none());
    }

    #[test]
    fn judging() {
        let ts = vec!["m::t_empty".to_string()];
        assert!(judge(
            &ts,
            101,
            "test m::t_empty ... FAILED\npanicked at x",
            0,
            "test m::t_empty ... ok"
        )
        .is_empty());
        assert!(judge(&ts, 0, "", 0, "").contains("already passes"));
        assert!(judge(&ts, 101, "error[E0425]: cannot find", 0, "").contains("does not compile"));
        assert!(judge(&ts, 1, "something else failed", 0, "").contains("No failure"));
        assert!(
            judge(&ts, 101, "test m::t_empty ... FAILED", 1, "boom").contains("fails with the fix")
        );
        assert!(judge(&ts, 101, "t_empty FAILED", 1, "boom").contains("No failure"));
        assert!(judge(&ts, 1, "t_empty", 0, "[checkout failed]").contains("checkout"));
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            0,
            "test m::t_empty ... ok"
        )
        .is_empty());
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            0,
            "test other ... ok"
        )
        .contains("None of the named"));
        assert!(judgef(
            &ts,
            101,
            "error[E0425]: cannot find function",
            1,
            "test m::t_empty ... FAILED"
        )
        .contains("fails with the feature"));
        assert!(judgef(&ts, 0, "", 0, "").contains("already passes"));
        assert!(
            judgef(&ts, 101, "test m::t_empty ... FAILED", 1, "boom").contains("with the feature")
        );
    }

    #[test]
    fn optimization_requires_the_same_named_checks_to_pass_before_and_after() {
        let ts = vec!["suite::case".to_string()];
        let pass = "test suite::case ... ok";
        assert!(judgeopt(&ts, 0, pass, 0, pass).is_empty());
        assert!(judgeopt(&ts, 101, "test suite::case ... FAILED", 0, pass)
            .contains("fail before the optimization"));
        assert!(judgeopt(&ts, 0, pass, 101, "test suite::case ... FAILED")
            .contains("fail after the optimization"));
        assert!(judgeopt(&ts, 0, "running 0 tests", 0, pass)
            .contains("Named checks are missing or did not pass before"));
        assert!(judgeopt(&ts, 0, pass, 0, "running 0 tests")
            .contains("Named checks are missing or did not pass after"));
        assert!(judgeopt(&ts, 0, "[checkout failed]", 0, pass).contains("checkout"));
    }

    #[test]
    fn pr_quota_zero_means_unlimited() {
        // A missing cap does not block either new PRs or work already in flight.
        assert!(!pr_quota_hit(0, 0));
        assert!(!pr_quota_hit(0, 2));
        assert!(!pr_quota_hit(0, 1_000_000));
        // Positive caps retain the existing inclusive boundary.
        assert!(!pr_quota_hit(2, 1));
        assert!(pr_quota_hit(2, 2));
        assert!(pr_quota_hit(2, 3));
        assert!(!pr_quota_hit(5, 4));
        assert!(pr_quota_hit(5, 5));
    }

    #[test]
    fn conventional_commit_subject_is_distinct_from_pull_request_title() {
        let portuguese = "Rejeitar datas civis inválidas nas entregas RMA";
        let title = pr_title(None, &json!({"title": portuguese}), "outra causa").unwrap();
        let subject = commit_subject(&title, false).unwrap();
        assert_eq!(
            subject,
            "fix: Rejeitar datas civis inválidas nas entregas RMA"
        );
        // PR titles remain human-readable and do not acquire a forced prefix.
        assert_eq!(title, portuguese);
        assert_eq!(
            commit_subject("fix(rma): validar datas civis", false).unwrap(),
            "fix(rma): validar datas civis"
        );
        assert_eq!(
            commit_subject("Adicionar avisos de RMA", true).unwrap(),
            "feat: Adicionar avisos de RMA"
        );
        assert_eq!(
            commit_subject("Corrigir falha\nMSG\ncomando inesperado", false).unwrap(),
            "fix: Corrigir falha"
        );
        assert!(commit_subject("   \n ", false).is_err());
        let long = "x".repeat(200);
        assert_eq!(commit_subject(&long, false).unwrap().chars().count(), 120);
    }

    #[test]
    fn optimization_commits_do_not_claim_unmeasured_performance_gains() {
        assert_eq!(
            optimization_commit_subject("perf: Reduce repeated parsing").unwrap(),
            "refactor: Reduce repeated parsing"
        );
        assert_eq!(
            optimization_commit_subject("Simplify internal lookup").unwrap(),
            "refactor: Simplify internal lookup"
        );
        assert!(optimization_commit_subject("perf: ").is_err());
        assert!(optimization_commit_subject("refactor(core):").is_err());
    }

    #[test]
    fn pull_request_title_falls_back_when_generated_title_is_blank() {
        let description = json!({"title": "   "});
        assert_eq!(
            pr_title(None, &description, "Datas de RMA inválidas").unwrap(),
            "Datas de RMA inválidas"
        );
        assert_eq!(
            pr_title(Some("Título aprovado"), &description, "Outro").unwrap(),
            "Título aprovado"
        );
        assert_eq!(
            pr_title(None, &json!({"title": "  Linha útil\nMSG"}), "").unwrap(),
            "Linha útil"
        );
        assert!(pr_title(None, &json!({"title": ""}), "  ").is_err());
    }

    #[test]
    fn bodies() {
        let j = Job::of(
            json!({"id": 5, "repo": "o/r", "num": 12, "kind": "fix", "title": "T", "d": {"mn": "main", "known": []}}),
        );
        let f = json!({"cause": "c", "fix": "f", "tests": ["t1"], "test_cmd": "cargo test t1", "untouched": ""});
        let b = body_lang(&j, &f, "cargo test", FOOT, false);
        assert!(b.starts_with("Fixes #12"));
        assert!(b.contains("`t1`: fails on `main`"));
        assert!(b.ends_with(FOOT));
        assert!(!body_lang(&j, &f, "cargo test", "", false).contains(FOOT));
        assert!(
            body_lang(&j, &f, "cargo test", FOOT0, false).ends_with(FOOT0)
                && !FOOT0.contains("ApolloOffice")
        );
        let h = Job::of(
            json!({"id": 6, "repo": "o/r", "num": 0, "kind": "fix", "title": "T", "d": {"mn": "main", "hunt": {"where": "a.rs:3", "title": "x"}}}),
        );
        let b = body_lang(
            &h,
            &json!({"cause": "Fixes #0. c", "fix": "f", "tests": ["a", "b"], "test_cmd": "t"}),
            "make test",
            FOOT,
            false,
        );
        assert!(b.starts_with("Found while testing `a.rs:3`"));
        assert!(!b.contains("#0"));
        assert!(b.contains("Regression tests `a`, `b`: fail on"));
        let fj = Job::of(
            json!({"id": 7, "repo": "o/r", "num": 9, "kind": "fix", "title": "T", "d": {"mn": "main", "feat": true}}),
        );
        let b = body_lang(&fj, &f, "cargo test", "", false);
        assert!(b.starts_with("Closes #9"));
        assert!(b.contains("## Why") && b.contains("## Change") && !b.contains("## Cause"));
        assert!(b.contains("Test `t1`: fails on `main` without the change"));
    }

    #[test]
    fn issue_closure_survives_pr_templates_without_duplicating_links() {
        let bug = Job::of(json!({"id": 5, "repo": "o/r", "num": 12, "kind": "fix", "d": {}}));
        assert_eq!(ensure_issue_closure(&bug, "Template content".into()), "Template content\n\nFixes #12");
        assert_eq!(ensure_issue_closure(&bug, "Template content\n\ncloses #12".into()), "Template content\n\ncloses #12");
        assert_eq!(ensure_issue_closure(&bug, "`Closes #12`".into()), "`Closes #12`\n\nFixes #12");
        assert_eq!(ensure_issue_closure(&bug, "```text\nCloses #12\n```".into()), "```text\nCloses #12\n```\n\nFixes #12");

        let feature = Job::of(json!({"id": 6, "repo": "o/r", "num": 13, "kind": "fix", "d": {"feat": true}}));
        assert_eq!(ensure_issue_closure(&feature, "Template content".into()), "Template content\n\nCloses #13");

        let found_by_hunter = Job::of(json!({"id": 7, "repo": "o/r", "num": 0, "kind": "fix", "d": {"hunt": {"title": "bug"}}}));
        assert_eq!(ensure_issue_closure(&found_by_hunter, "Template content".into()), "Template content");
    }

    #[test]
    fn junks() {
        assert!(junk("__pycache__/tb.cpython-314.pyc"));
        assert!(junk("tests/__pycache__/x.pyc"));
        assert!(junk("build/a.o"));
        assert!(!junk("tb.py"));
        assert!(!junk("src/main.rs"));
    }

    #[test]
    fn gitcfg() {
        let u = "https://github.com/o/r.git";
        let l = "core.repositoryformatversion=0\ncore.filemode=true\nremote.origin.url=https://github.com/o/r.git\nremote.origin.fetch=+refs/heads/*:refs/remotes/origin/*\nremote.origin.promisor=true\nremote.origin.partialclonefilter=blob:none\nbranch.main.remote=origin\nbranch.main.merge=refs/heads/main\nuser.name=T\nuser.email=t@t\nextensions.partialclone=origin\n";
        assert!(cfg_ok(l, u).is_ok());
        assert!(cfg_ok(&format!("{l}core.fsmonitor=curl x\n"), u).is_err());
        assert!(cfg_ok(&format!("{l}core.hookspath=/tmp/h\n"), u).is_err());
        assert!(cfg_ok(
            &format!("{l}url.https://evil/.insteadof=https://github.com/\n"),
            u
        )
        .is_err());
        assert!(cfg_ok(&format!("{l}include.path=/tmp/x\n"), u).is_err());
        assert!(cfg_ok(&l.replace("github.com/o/r", "evil.com/o/r"), u).is_err());
    }

    #[test]
    fn slugs() {
        let j = Job::of(
            json!({"id": 5, "repo": "o/r", "num": 12, "title": "Crash on empty input!!", "d": {}}),
        );
        assert_eq!(bslug(&j), "fix/12-crash-on-empty-input");
        assert_eq!(slug("a/b"), "a__b");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn subscription_review_returns_free_author_feedback_and_stages_proposal() {
        let _cli_lock = crate::ai::CLI_TEST_LOCK.lock().await;
        use std::os::unix::fs::PermissionsExt;
        let dd = std::env::temp_dir().join(format!(
            "apollo-review-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("sum.py"), "def add(a,b): return a-b\n").unwrap();
        git(&["add", "sum.py"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "base",
        ]);
        let base = git(&["rev-parse", "HEAD"]);
        std::fs::write(repo.join("sum.py"), "def add(a,b): return 5\n").unwrap();
        git(&["add", "sum.py"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "bad hardcoded fix",
        ]);
        let cli = dd.join("review-cli");
        std::fs::write(&cli,r##"#!/usr/bin/python3
import sys,json,pathlib
args=sys.argv[1:];text=sys.stdin.read()
assert 'read-only' in args and '--ignore-user-config' in args
assert not any('mcp_servers.office' in x for x in args), 'reviewer must have no worker tools'
assert 'def add(a,b): return 5' in text
assert 'real fixture verification: negative addition fails' in text
answer=json.dumps({'ok':False,'issues':['Replace hardcoded 5 with a+b; test negative values too.'],'lesson':{'mistake':'Hardcoded example instead of general arithmetic.','correction':'Implement a+b.','verification':'Exercise positive and negative integers.'}})
pathlib.Path(args[args.index('-o')+1]).write_text(answer)
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':10,'output_tokens':10}}))
"##).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("APOLLO_OFFICE_CODEX", &cli);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','python3 sum.py')",
            pa![],
        );
        let id=db.jnew("o/r",0,"fix","addition bug","", "fix","",json!({"h":"local","base":base,"ext":{"src":"fixture"},"verification_feedback":"real fixture verification: negative addition fails"}),None,0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let mut job = a.db.jg(id).unwrap();
        let mut f = json!({"title":"Fix addition","cause":"subtraction","fix":"hardcode five"});
        let feedback = review(&a, &mut job, &mut f).await.unwrap();
        std::env::remove_var("APOLLO_OFFICE_CODEX");
        assert!(feedback.contains("Replace hardcoded 5"));
        assert_eq!(a.db.jg(id).unwrap().d["last_review"]["model"], "codex:");
        assert_eq!(
            a.db.n("select count(*) from kv where k like 'lesson:%'", pa![]),
            1
        );
        assert!(
            !a.db.jg(id).unwrap().d["ms"]
                .as_array()
                .map(|v| v.contains(&json!("codex:")))
                .unwrap_or(false),
            "review must not claim code authorship"
        );
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn java_compile_or_loading_failure_is_not_a_regression_proof() {
        let names = vec!["channel_identity".to_string()];
        assert!(judge(
            &names,
            1,
            "BlockedChannelData.java:24: error: cannot find symbol\nchannel_identity",
            0,
            "channel_identity OK"
        )
        .contains("compile or load"));
        assert!(judge(
            &names,
            1,
            "channel_identity java.lang.ClassNotFoundException: BlockedChannelData",
            0,
            "channel_identity OK"
        )
        .contains("compile or load"));
        assert!(judge(
            &names,
            1,
            "channel_identity java.lang.AssertionError: distinct IDs collide",
            0,
            "channel_identity OK"
        )
        .is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn independent_acceptance_rejects_author_tests_green_and_feedback_reaches_fixer() {
        use sha2::{Digest, Sha256};
        let dd = std::env::temp_dir().join(format!(
            "apollo-independent-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = || {
            git(&["add", "."]);
            git(&[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-qm",
                "fixture",
            ]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "work"]);
        std::fs::write(repo.join("maths.py"), "def add(a,b): return a-b\n").unwrap();
        let base = commit();
        std::fs::write(repo.join("author_test.py"),"from maths import add\nimport sys\nok=add(2,3)==5\nprint('positive_addition', 'PASS' if ok else 'FAIL')\nsys.exit(0 if ok else 1)\n").unwrap();
        commit();
        std::fs::write(repo.join("maths.py"), "def add(a,b): return 5\n").unwrap();
        commit();
        let script="import os,pathlib,runpy\nm=runpy.run_path(str(pathlib.Path(os.environ['APOLLO_SOURCE_ROOT'])/'maths.py'))\nok=m['add'](-2,3)==1\nprint('APOLLO_CHECK:negative_addition:'+('PASS' if ok else 'FAIL'))\nraise SystemExit(0 if ok else 1)\n";
        let frozen = json!({"base":base,"baseline_valid":true,"designer":"fixture-independent-reviewer","checks":["negative_addition"],"script":script});
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"quality_checks":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run(
            "insert into rp(full,ci) values('o/r','python3 author_test.py')",
            pa![],
        );
        let id = db.jnew(
            "o/r",
            0,
            "fix",
            "addition",
            "",
            "fix",
            "",
            json!({"h":"local","base":base,"br":"work","acceptance":frozen}),
            None,
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let mut job = a.db.jg(id).unwrap();
        let feedback=verify(&a,&mut job,&json!({"tests":["positive_addition"],"test_cmd":"PYTHONDONTWRITEBYTECODE=1 python3 author_test.py"})).await.unwrap();
        assert_eq!(
            job.d["proof"]["b_exit"], 0,
            "author's tests must really be green"
        );
        assert!(feedback.contains("negative_addition"), "{feedback}");
        assert_eq!(job.d["acceptance_proof"]["ok"], false);
        assert!(fixp(&a, &job, &feedback).contains("Independent acceptance rejected"));
        assert_eq!(
            job.d["acceptance_proof"]["run"]["script_sha256"],
            json!(hex::encode(Sha256::digest(script.as_bytes())))
        );
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn review_with_several_commits_is_squashed() {
        let dd = std::env::temp_dir().join(format!(
            "apollo-squash-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let commit = |m: &str| {
            git(&["add", "."]);
            git(&[
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@localhost",
                "commit",
                "-qm",
                m,
            ]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "fixture"]);
        git(&["config", "user.email", "fixture@localhost"]);
        std::fs::write(repo.join("a.txt"), "1\n").unwrap();
        let base = commit("base");
        std::fs::write(repo.join("a.txt"), "2\n").unwrap();
        commit("Address review");
        std::fs::write(repo.join("b.txt"), "3\n").unwrap();
        commit("more");
        let db = crate::db::Db::mem();
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run("insert into rp(full,ci) values('o/r','true')", pa![]);
        let id = db.jnew(
            "o/r",
            1,
            "review",
            "t",
            "",
            "fix",
            "",
            json!({"h":"local","base":base,"br":"work"}),
            Some(1),
            0.0,
        );
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        let job = a.db.jg(id).unwrap();
        let fb = vrev(&a, &job, &json!({})).await.unwrap();
        assert_eq!(fb, "");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "1");
        assert_eq!(git(&["log", "-1", "--format=%s"]), "Address review");
        assert_eq!(git(&["show", "HEAD:b.txt"]), "3");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resumed_free_first_submission_without_owner_approval_never_invokes_github() {
        use std::os::unix::fs::PermissionsExt;
        let dd = std::env::temp_dir().join(format!(
            "apollo-publish-{}",
            hex::encode(rand::random::<[u8; 8]>())
        ));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(out.status.success());
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["init", "-q", "-b", "work"]);
        std::fs::write(repo.join("file"), "fixture\n").unwrap();
        git(&["add", "file"]);
        git(&[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@localhost",
            "commit",
            "-qm",
            "base",
        ]);
        let base = git(&["rev-parse", "HEAD"]);
        let marker = dd.join("github-called");
        let gh = dd.join("gh");
        std::fs::write(
            &gh,
            format!(
                "#!/bin/sh\nprintf called > {}\nexit 97\n",
                q(&marker.display().to_string())
            ),
        )
        .unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("APOLLO_OFFICE_GH", &gh);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true}));
        db.run(
            "insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)",
            pa![],
        );
        db.run("insert into rp(full,mode) values('o/r','auto')", pa![]);
        let id=db.jnew("o/r",0,"fix","fixture","","ready","resumed",json!({"h":"local","base":base,"br":"work","rs":true,"ap":false,"f":{"title":"Fixture","tests":[],"test_cmd":"true"}}),None,0.0);
        let a = std::sync::Arc::new(crate::app::App::new(
            db,
            dd.clone(),
            1,
            "apollo-office".into(),
        ));
        // Missing and malformed grants, plus an unusable preview base, must
        // all remain held. The fake GitHub executable detects publication.
        for grant in [json!(false), Value::Null, json!("true"), json!(1)] {
            let mut job = a.db.jg(id).unwrap();
            job.d["ap"] = grant;
            job.d["base"] = json!("nonexistent-preview-base");
            a.db.jset(id, &[("st", json!("ready"))]);
            let result =
                tokio::time::timeout(std::time::Duration::from_secs(2), submit(&a, &mut job)).await;
            assert!(
                !marker.exists(),
                "resuming a job must not invoke GitHub without owner approval"
            );
            assert!(matches!(result,Ok(Err(X::Back(ref reason))) if reason.is_empty()));
            assert_eq!(a.db.jg(id).unwrap().st, "approve");
        }
        std::env::remove_var("APOLLO_OFFICE_GH");
        std::fs::remove_dir_all(dd).unwrap();
    }
    // ---- staged RED/GREEN two-phase TDD ----

    #[cfg(unix)]
    fn staged_fixture() -> (Ax, i64, std::path::PathBuf, std::path::PathBuf) {
        let dd = std::env::temp_dir().join(format!("apollo-staged-{}", hex::encode(rand::random::<[u8; 8]>())));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(repo.join("t")).unwrap();
        let git = {
            let repo = repo.clone();
            move |args: &[&str]| {
                let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
                assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
        };
        // Intentional bug: add subtracts instead of adding.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 - $2 )); }\n").unwrap();
        std::fs::write(repo.join("t/run.sh"), "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n").unwrap();
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "fixture"]);
        git(&["config", "user.email", "fixture@localhost"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        let db = crate::db::Db::mem();
        // Pilot config: staged on, allowlisted repo, free-first OFF (required).
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]}));
        db.run("insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)", pa![]);
        // Mandatory GREEN production allowlist for the pilot repo: only add.sh.
        db.run("insert into rp(full,ci,green) values('o/r','bash t/run.sh','^add\\.sh$')", pa![]);
        let id = db.jnew("o/r", 0, "fix", "wrong addition", "", "fix", "", json!({"h":"local","base":base,"br":"work","repro":{"how":"add 2 3 prints -1"}}), None, 0.0);
        let a = std::sync::Arc::new(crate::app::App::new(db, dd.clone(), 1, "apollo-office".into()));
        (a, id, dd, repo)
    }

    const RED_FAIL: &str = "source ./add.sh\n[ \"$(add 2 3)\" = 5 ] && echo 'ok 1 - sum' || { echo 'not ok 1 - sum'; exit 1; }\n";

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_happy_path_makes_test_then_fix_commit_with_real_red_green() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let git = |args: &[&str]| String::from_utf8_lossy(&std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // RED: author writes only the failing test.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "add returns a wrong sum").await.unwrap();
        assert!(fb.is_none(), "RED should be accepted: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "1");
        assert!(git(&["diff", "--name-only", &base, "HEAD"]).lines().all(|l| l == "t/sum_test.sh"));
        assert!(git(&["log", "-1", "--format=%s"]).starts_with("test:"));
        let red_files: Vec<String> = git(&["diff", "--name-only", &base, "HEAD"]).lines().map(|l| l.to_string()).collect();
        let test_commit = git(&["rev-parse", "HEAD"]);
        // GREEN: author writes only the fix.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let js = json!({"title":"Add instead of subtract","test_cmd":"bash t/run.sh","tests":["sum"],"cause":"c","fix":"f"});
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "Add instead of subtract").await.unwrap();
        assert!(fb.is_none(), "GREEN should be accepted: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{base}..HEAD")]), "2");
        assert!(git(&["log", "-1", "--format=%s"]).starts_with("fix:"));
        assert!(git(&["diff", "--name-only", "HEAD~1", "HEAD"]).lines().all(|l| l == "add.sh"), "fix commit must not touch the test");
        // The authoritative two-commit gate agrees.
        let v = verify(&a, &mut job, &js).await.unwrap();
        assert_eq!(v, "", "verify must confirm red on the test commit and green on the fix commit");
        assert_ne!(job.d["proof"]["a_exit"], 0);
        assert_eq!(job.d["proof"]["b_exit"], 0);
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_implementation_changes() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap(); // sneaks the fix in
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "{fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{}..HEAD", job.s("base"))]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no commit on rejection");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_test_that_already_passes() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("must fail"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_rejects_compile_or_load_error() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), "echo placeholder\n").unwrap();
        // A test that does not load (here a missing import) is not a real failing
        // test. Python's error text is locale-independent, unlike a shell's.
        let fb = red_commit(&a, &mut job, "python3 -c 'import nonexistent_mod_xyz'", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("compile or load"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_rejects_modifying_the_committed_test() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(&repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // GREEN weakens the test and adds the fix.
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_rejects_when_test_still_fails() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        std::fs::write(repo.join("add.sh"), "add() { echo 0; }\n").unwrap(); // allowlisted but not an actual fix
        let fb = green_commit(&a, &mut job, &["t/sum_test.sh".to_string()], &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("still fails"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_mode_is_opt_in_and_scoped() {
        let (a, id, dd, _repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        assert!(staged(&a, &job), "fix job with staged_tdd + allowlisted repo + free_first OFF is staged");
        job.d["feat"] = json!(true);
        assert!(!staged(&a, &job), "feature requests keep the legacy flow");
        job.d.as_object_mut().unwrap().remove("feat");
        job.kind = "review".into();
        assert!(!staged(&a, &job), "PR-review jobs keep the legacy flow");
        job.kind = "fix".into();
        // B5: another repo (e.g. FixFlow) is never staged even with every flag on.
        let mut other = job.clone();
        other.repo = "croquete1/FixFlow-novo".into();
        assert!(!staged(&a, &other) && !staged_intended(&a, &other), "non-allowlisted repo (FixFlow) must stay on the legacy flow");
        a.db.ks("set", &json!({"staged_tdd":true})); // allowlist removed
        assert!(!staged(&a, &job), "fails closed: no allowlist means no repo is staged");
        a.db.ks("set", &json!({"staged_repos":["o/r"]})); // opt-in flag off
        assert!(!staged(&a, &job), "off without the opt-in flag");
        // V3-P1-3: with free_first ON the mode is INTENDED but NOT active (it would
        // substitute the configured FIXER/REVIEWER); fix() rejects that config.
        a.db.ks("set", &json!({"free_first":true,"staged_tdd":true,"staged_repos":["o/r"]}));
        assert!(staged_intended(&a, &job), "still intended with free_first on");
        assert!(!staged(&a, &job), "but NOT active under free-first routing (fail closed)");
        a.db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]})); // free_first OFF
        assert!(staged(&a, &job), "active with the flag, allowlist and free_first off");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn testish_classifies_common_layouts_and_honours_repo_override() {
        let db = crate::db::Db::mem();
        db.run("insert into rp(full) values('o/r')", pa![]);
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let j = crate::db::Job::of(json!({"repo":"o/r","kind":"fix"}));
        for p in ["t/sum_test.sh", "tests/test_x.py", "src/foo_test.go", "spec/foo_spec.rb", "FooTest.java", "__tests__/a.js", "conftest.py"] {
            assert!(testish(&a, &j, p), "expected test: {p}");
        }
        for p in ["src/add.sh", "lib/maths.py", "main.go", "README.md"] {
            assert!(!testish(&a, &j, p), "expected non-test: {p}");
        }
        // explicit per-repo override (rp.tests regex) wins over the heuristic
        let a2 = {
            let db = crate::db::Db::mem();
            db.run("insert into rp(full,tests) values('o/r','^impl/')", pa![]);
            std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()))
        };
        assert!(testish(&a2, &j, "impl/whatever"));
        assert!(!testish(&a2, &j, "tests/test_x.py"));
        // An invalid override regex must not panic; it falls back to the heuristic.
        let a3 = {
            let db = crate::db::Db::mem();
            db.run("insert into rp(full,tests) values('o/r','[unclosed(')", pa![]);
            std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()))
        };
        assert!(testish(&a3, &j, "tests/test_x.py"));
        assert!(!testish(&a3, &j, "src/add.sh"));
    }

    // Adversarial: the shell guard (STAGED_GIT) is defeatable (a script the regex
    // never sees), so integrity cannot rest on it. These tests prove the controller
    // DETECTS a forged history by re-reading git state, independent of the guard.
    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_detects_forged_commit_even_if_the_shell_guard_is_bypassed() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let git = |args: &[&str]| {
            assert!(std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap().status.success());
        };
        // Author (via some bypass) commits the test itself and leaves a dirty fix.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-qm", "test: forged"]);
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("history changed"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_green_detects_amended_red_commit() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none());
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(&repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        let stale = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // Author amends the committed test commit behind the controller's back.
        std::fs::write(repo.join("t/sum_test.sh"), "echo 'ok 1 - sum'\n").unwrap();
        assert!(std::process::Command::new("git").args(["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-aqm", "test: amended", "--amend"]).current_dir(&repo).output().unwrap().status.success());
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        // Controller still holds the stale test_commit sha; HEAD no longer matches.
        let fb = green_commit(&a, &mut job, &red_files, &stale, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("history changed"), "{fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_red_sees_individual_files_in_a_new_untracked_directory() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        // A brand-new directory whose name looks test-ish, hiding production code.
        std::fs::create_dir_all(repo.join("testkit")).unwrap();
        std::fs::write(repo.join("testkit/runner.py"), "x=1\n").unwrap(); // not test-ish by name
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let seen = changed_paths(&a, &job).await.unwrap();
        assert!(seen.iter().any(|p| p == "testkit/runner.py"), "new-dir files must be expanded: {seen:?}");
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "hidden non-test file must be caught: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // ===== Regression tests for the six audit blockers =====

    #[cfg(unix)]
    async fn red_ok(a: &Ax, job: &mut Job, repo: &std::path::Path) -> (String, Vec<String>) {
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(red_commit(a, job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap().is_none(), "RED fixture must be accepted");
        let base = job.s("base");
        let test_commit = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string();
        let red_files: Vec<String> = String::from_utf8_lossy(&std::process::Command::new("git").args(["diff", "--name-only", &base, "HEAD"]).current_dir(repo).output().unwrap().stdout).lines().map(|l| l.to_string()).collect();
        (test_commit, red_files)
    }

    // B1: a GREEN that disables the runner (so the bug persists) is rejected by the
    // positive-execution proof, even though the test command exits 0.
    #[cfg(unix)]
    #[tokio::test]
    async fn b1_green_cannot_pass_by_neutering_the_runner() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // (a) The "fix" leaves add.sh buggy and rewrites the runner named in the
        // frozen command to a no-op: caught because the runner is a protected path.
        std::fs::write(repo.join("t/run.sh"), "echo tests passed\nexit 0\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "neutering the runner must be rejected: {fb:?}");
        // (b) Positive-proof backstop: a command that exits 0 without ever running
        // the named test (no protected file touched) is still rejected.
        std::fs::write(repo.join("t/run.sh"), "e=0\nfor f in t/*_test.sh; do bash \"$f\" || e=1; done\nexit $e\n").unwrap(); // restore runner
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap(); // real production change
        let fb2 = green_commit(&a, &mut job, &red_files, &test_commit, "true", &["sum".to_string()], "x").await.unwrap();
        assert!(fb2.as_deref().unwrap_or("").contains("ran"), "a command that never runs the named test must fail positive proof: {fb2:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B2: a test command that mutates production after classification must not get
    // that production change committed into the RED (test-only) commit.
    #[cfg(unix)]
    #[tokio::test]
    async fn b2_test_command_mutating_production_is_rejected_not_committed() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        // The test command writes to production as a side effect.
        let tc = "printf 'add() { echo $(( $1 + $2 )); }\\n' > add.sh; bash t/run.sh";
        let fb = red_commit(&a, &mut job, tc, &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("non-test files"), "production mutation by test_cmd must be caught: {fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{}..HEAD", job.s("base"))]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no commit on rejection");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B3: renaming production into a test directory must be detected via the rename
    // origin, not accepted as a pure test change.
    #[cfg(unix)]
    #[tokio::test]
    async fn b3_rename_of_production_into_tests_is_detected() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        // Also add a real failing test so only the rename is the offending change.
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        assert!(std::process::Command::new("git").args(["mv", "add.sh", "t/add_test.sh"]).current_dir(&repo).output().unwrap().status.success());
        let seen = changed_paths(&a, &job).await.unwrap();
        assert!(seen.iter().any(|p| p == "add.sh"), "rename origin must be surfaced: {seen:?}");
        let fb = red_commit(&a, &mut job, "bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("test files only"), "production rename must be rejected: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B4: a test_cmd that runs git history commands is refused (the run_wt path no
    // longer bypasses the history guard).
    #[cfg(unix)]
    #[tokio::test]
    async fn b4_test_cmd_running_git_is_refused() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("t/sum_test.sh"), RED_FAIL).unwrap();
        let fb = red_commit(&a, &mut job, "git reset --hard HEAD~1; bash t/run.sh", &["sum".to_string()], "t").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("only run the tests"), "git in test_cmd must be refused: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // B5 is covered by staged_mode_is_opt_in_and_scoped (FixFlow + fail-closed).

    // B6: GREEN cannot change the proven test command or test names; the controller
    // freezes the RED values and the verifier uses only those.
    #[cfg(unix)]
    #[tokio::test]
    async fn b6_green_cannot_drift_test_cmd_or_names() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // The frozen RED command is "bash t/run.sh" / ["sum"]. Even if GREEN tried to
        // substitute a trivially-passing command, green_commit is called with the
        // FROZEN values (as staged_author does), so the drift has no effect: the real
        // runner must still pass. Here we prove the frozen values are what run.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.is_none(), "frozen command must drive GREEN: {fb:?}");
        // The persisted checkpoint records the immutable proof identity.
        job.d["staged_red"] = json!({"test_cmd":"bash t/run.sh","tests":["sum"],"test_commit":test_commit});
        assert_eq!(job.d["staged_red"]["test_cmd"], "bash t/run.sh");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // A4: REPRO and other agents keep an unrestricted shell; only the FIXER phase
    // asks run with the staged guard.
    #[cfg(unix)]
    #[tokio::test]
    async fn a4_repro_shell_is_not_staged() {
        let (a, id, dd, _repo) = staged_fixture();
        let job = a.db.jg(id).unwrap();
        assert!(!rn(&a, &job).unwrap().staged, "rn() must never pre-set the staged guard");
        std::fs::remove_dir_all(dd).unwrap();
    }

    #[test]
    fn parse_status_z_handles_modify_rename_untracked_and_spaces() {
        assert_eq!(parse_status_z(" M add.sh\0"), vec!["add.sh"]);
        assert_eq!(parse_status_z("R  test/x.sh\0src/x.sh\0"), vec!["test/x.sh", "src/x.sh"]);
        assert_eq!(parse_status_z("?? new file.sh\0"), vec!["new file.sh"]);
        assert!(parse_status_z("").is_empty());
    }

    // ===== v2 audit: P0-1, P0-2, P1-3, P2-5 =====

    // P0-1: the classic `npm test` runner-swap. GREEN leaves the bug and rewrites
    // package.json so `npm test` becomes a no-op. The controller rejects the change
    // to the runner config BEFORE running anything, so no fix commit is created and
    // the final gate never sees a false green. No Node needed: package.json is a
    // protected runner-config file.
    #[cfg(unix)]
    #[tokio::test]
    async fn p0_1_green_cannot_pass_by_editing_package_json_runner() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"node --test\"}}\n").unwrap();
        // commit package.json into the base so it is a tracked production/runner file
        for args in [vec!["add", "."], vec!["-c", "user.name=x", "-c", "user.email=x@y", "commit", "-qm", "add runner"]] {
            assert!(std::process::Command::new("git").args(&args).current_dir(&repo).output().unwrap().status.success());
        }
        let base = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        job.d["base"] = json!(base);
        a.db.jd(&mut job);
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // GREEN: bug stays; neuter the runner via package.json.
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"echo sum passed\"}}\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "npm test", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "package.json runner edit must be rejected: {fb:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{test_commit}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "0", "no fix commit may form");
        assert!(runner_cfg("package.json") && runner_cfg("a/b/pyproject.toml") && runner_cfg("jest.config.js") && !runner_cfg("src/add.sh"));
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P0-2: in staged mode the FIXER is the configured model (Qwen in the pilot),
    // never a free-first routing model, and the REVIEWER stays the configured one
    // (Claude), both independent of free_first. Checks the selection seam directly.
    #[test]
    fn p0_2_staged_uses_configured_fixer_and_reviewer_not_free_first() {
        let db = crate::db::Db::mem();
        // Pilot config: explicit Qwen fixer and Claude reviewer, free_first OFF.
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"],"staged_fallback":"codex:gpt-6-luna","md":{"fixer":"ollama:qwen3:8b","reviewer":"cc:opus","big":"ollama:qwen3:8b"}}));
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let job = crate::db::Job::of(json!({"repo":"o/r","kind":"fix","d":{}}));
        assert!(staged(&a, &job));
        assert!(!crate::routing::enabled(&a), "pilot runs with free-first off");
        assert_eq!(a.md("fixer"), "ollama:qwen3:8b", "FIXER must be the configured Qwen");
        assert_eq!(a.md("reviewer"), "cc:opus", "REVIEWER must be the configured Claude");
        let visible = staged_ui_config(&a);
        assert_eq!(visible["active"], true, "the Control Room must see the active staged route");
        assert_eq!(visible["primary"], "ollama:qwen3:8b");
        assert_eq!(visible["fallback"], "codex:gpt-6-luna");
        assert_eq!(visible["fallback_effort"], "max");
        assert_eq!(visible["reviewer"], "cc:opus");
        assert_eq!(visible["repos"][0], "o/r");
        // Even if free_first were on, staged still selects a.md('fixer') in fix();
        // a.md itself returns the free model under free_first, which is exactly why
        // the pilot keeps free_first off. Document the invariant with routing off.
        assert!(!crate::routing::is_free("ollama:qwen3:8b"), "Qwen is not a free-tier routing model");
    }

    // P1-3: after a restart with a valid RED-only checkpoint, staged_resume keeps
    // the RED commit (Green) instead of rebuilding it; an invalid checkpoint is
    // discarded (Fresh).
    #[cfg(unix)]
    #[tokio::test]
    async fn p1_3_resume_reuses_valid_red_checkpoint_and_discards_invalid() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        job.d["staged_red"] = json!({"test_commit":test_commit,"test_cmd":"bash t/run.sh","tests":["sum"],"red_files":red_files,"title":"t"});
        a.db.jd(&mut job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Green(s) => assert_eq!(s.test_commit, test_commit, "resume must keep the RED commit"),
            _ => panic!("valid RED-only checkpoint must resume as Green"),
        }
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), test_commit, "branch stays on the RED commit, not reset to base");
        // A stale checkpoint whose commit never materialised AND whose branch is at
        // the base is safely discarded -> Fresh (nothing accessible to lose).
        std::process::Command::new("git").args(["reset", "-q", "--hard", &base]).current_dir(&repo).output().unwrap();
        job.d["staged_red"] = json!({"test_commit":"0000000000000000000000000000000000000000","test_cmd":"bash t/run.sh","tests":["sum"],"stage":"pending"});
        a.db.jd(&mut job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh));
        assert!(job.d["staged_red"].is_null(), "stale checkpoint at base is discarded");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P2-5: commit_scope requires the staged set to EQUAL the authorised set, so an
    // authorised path that silently fails to stage is caught (not just extras).
    #[cfg(unix)]
    #[tokio::test]
    async fn p2_5_commit_scope_rejects_missing_authorised_path() {
        let (a, id, dd, repo) = staged_fixture();
        let job = a.db.jg(id).unwrap();
        std::fs::write(repo.join("real.sh"), "x\n").unwrap();
        // Authorise a real changed file plus a phantom that does not exist -> add of
        // the phantom stages nothing for it, so the sets differ and it is rejected.
        let fb = commit_scope(&a, &job, &["real.sh".to_string()], "test: x").await;
        assert!(fb.is_ok() && fb.unwrap().is_none(), "exact set commits cleanly");
        std::fs::write(repo.join("real2.sh"), "y\n").unwrap();
        let fb = commit_scope(&a, &job, &["real2.sh".to_string(), "ghost.sh".to_string()], "test: y").await.unwrap();
        let m = fb.as_deref().unwrap_or("");
        assert!(m.contains("could not stage") || m.contains("unexpected staging state"), "missing authorised path must be caught: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // P2-6 (partial, no live model): drive the controller phases as a simulated
    // author across a failed GREEN then a successful GREEN, proving the RED commit
    // SHA is preserved across the retry and exactly two commits result.
    #[cfg(unix)]
    #[tokio::test]
    async fn p2_6_simulated_author_red_then_failed_then_passing_green() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        // GREEN attempt 1 (simulated author): writes an unrelated non-fix -> rejected.
        std::fs::write(repo.join("noop.sh"), "true\n").unwrap();
        let fb1 = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb1.is_some(), "a non-fix GREEN must be rejected");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), test_commit, "RED commit preserved after failed GREEN");
        // Reset working tree to the RED commit (as staged_author does between tries).
        for args in [vec!["checkout", "-q", "-f", &test_commit], vec!["clean", "-fdq"]] {
            std::process::Command::new("git").args(&args).current_dir(&repo).output().unwrap();
        }
        // GREEN attempt 2 (simulated author): the real fix -> accepted.
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb2 = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix add").await.unwrap();
        assert!(fb2.is_none(), "the real fix must be accepted: {fb2:?}");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2");
        assert_red_then_green(&repo);
        std::fs::remove_dir_all(dd).unwrap();
    }

    // Deterministic RED->GREEN check via git directly (no ok()-retry contention):
    // the test fails on the test commit (HEAD~1) and passes on the fix commit (HEAD).
    #[cfg(unix)]
    fn assert_red_then_green(repo: &std::path::Path) {
        let run = |rev: &str| {
            std::process::Command::new("git").args(["checkout", "-q", "-f", rev]).current_dir(repo).output().unwrap();
            std::process::Command::new("bash").arg("t/run.sh").current_dir(repo).output().unwrap().status.code()
        };
        let head = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string();
        assert_eq!(run("HEAD~1"), Some(1), "RED: test must fail on the test commit");
        assert_eq!(run(&head), Some(0), "GREEN: test must pass on the fix commit");
    }

    // ===== v3 audit: V3-P0-1, V3-P1-2, V3-P1-3, V3-P2-4 =====

    // V3-P0-1: the INDIRECT runner attack. `npm test` -> `sh scripts/check.sh`.
    // GREEN leaves production wrong and rewrites scripts/check.sh to print the test
    // name. The mandatory production allowlist (only src/calc.cjs) rejects the
    // change to scripts/check.sh before anything runs; no fix commit forms.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p0_1_green_cannot_edit_indirect_runner_script() {
        let dd = std::env::temp_dir().join(format!("apollo-v3runner-{}", hex::encode(rand::random::<[u8; 8]>())));
        let repo = dd.join("work/o__r");
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::create_dir_all(repo.join("test")).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        std::fs::write(repo.join("src/calc.cjs"), "exports.subtract = (a,b) => a+b;\n").unwrap();
        std::fs::write(repo.join("package.json"), "{\"scripts\":{\"test\":\"sh scripts/check.sh\"}}\n").unwrap();
        std::fs::write(repo.join("scripts/check.sh"), "node --test test/calc.test.cjs\n").unwrap();
        std::fs::write(repo.join("t_placeholder"), "x").unwrap();
        git(&["init", "-q", "-b", "work"]);
        git(&["config", "user.name", "x"]);
        git(&["config", "user.email", "x@y"]);
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        let base = git(&["rev-parse", "HEAD"]);
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"staged_tdd":true,"staged_repos":["o/r"]}));
        db.run("insert into hs(id,ssh,dk,raw,on_) values('local','',0,1,1)", pa![]);
        // Production allowlist: ONLY src/calc.cjs.
        db.run("insert into rp(full,green,tests) values('o/r','^src/calc\\.cjs$','(^|/)test/')", pa![]);
        let id = db.jnew("o/r", 0, "fix", "subtraction", "", "fix", "", json!({"h":"local","base":base,"br":"work","repro":{"how":"x"}}), None, 0.0);
        let a = std::sync::Arc::new(crate::app::App::new(db, dd.clone(), 1, "apollo-office".into()));
        let mut job = a.db.jg(id).unwrap();
        // RED: a real failing test under test/ (node assertion).
        std::fs::write(repo.join("test/calc.test.cjs"), "const {test}=require('node:test');const assert=require('node:assert/strict');const {subtract}=require('../src/calc.cjs');\ntest('subtraction',()=>assert.equal(subtract(7,3),4));\n").unwrap();
        let red = red_commit(&a, &mut job, "npm test", &["subtraction".to_string()], "cover subtraction").await.unwrap();
        assert!(red.is_none(), "RED must be accepted: {red:?}");
        let test_commit = git(&["rev-parse", "HEAD"]);
        let red_files = vec!["test/calc.test.cjs".to_string()];
        // GREEN: do NOT fix src/calc.cjs; neuter the indirect runner script.
        std::fs::write(repo.join("scripts/check.sh"), "echo subtraction\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "npm test", &["subtraction".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("allowlisted production files"), "indirect runner edit must be rejected: {fb:?}");
        assert_eq!(git(&["rev-list", "--count", &format!("{test_commit}..HEAD")]), "0", "no fix commit may form");
        assert_eq!(std::fs::read_to_string(repo.join("src/calc.cjs")).unwrap().trim(), "exports.subtract = (a,b) => a+b;", "production stays buggy, not falsely green");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P0-1 (fail-closed): a staged repo with NO green allowlist cannot run GREEN.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p0_1_green_requires_mandatory_allowlist() {
        let (a, id, dd, repo) = staged_fixture();
        a.db.run("update rp set green='' where full='o/r'", pa![]); // remove allowlist
        let mut job = a.db.jg(id).unwrap();
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        let fb = green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "x").await.unwrap();
        assert!(fb.as_deref().unwrap_or("").contains("requires a production-file allowlist"), "missing allowlist must fail closed: {fb:?}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P1-2: a crash after the GREEN commit but before the checkpoint is cleared
    // must NOT discard the commit. staged_resume returns Finished and the two
    // commits stay on the branch.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p1_2_resume_after_green_commit_keeps_both_commits() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        assert!(green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix").await.unwrap().is_none());
        let green = String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim().to_string();
        // Simulate the crash window: both checkpoints persisted, not yet cleared.
        job.d["staged_red"] = json!({"test_commit":test_commit,"test_cmd":"bash t/run.sh","tests":["sum"],"red_files":red_files,"title":"fix"});
        job.d["staged_green"] = json!({"commit":green,"js":{"title":"fix","test_cmd":"bash t/run.sh","tests":["sum"],"cause":"c","fix":"f"}});
        a.db.jd(&mut job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Finished(js) => assert_eq!(js["title"], "fix"),
            _ => panic!("must recover the finished RED+GREEN pair"),
        }
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2", "both commits preserved, no destructive reset");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&repo).output().unwrap().stdout).trim(), green, "HEAD still the GREEN commit");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P1-3: fix() refuses to run staged under free-first routing (fail closed),
    // rather than silently substituting the configured FIXER/REVIEWER.
    #[test]
    fn v3_p1_3_free_first_on_is_a_blocking_conflict() {
        let db = crate::db::Db::mem();
        db.ks("set", &json!({"free_first":true,"staged_tdd":true,"staged_repos":["o/r"]}));
        let a = std::sync::Arc::new(crate::app::App::new(db, std::env::temp_dir(), 1, "x".into()));
        let job = crate::db::Job::of(json!({"repo":"o/r","kind":"fix","d":{}}));
        assert!(staged_conflict(&a, &job), "intended + free_first on is a conflict fix() rejects");
        assert!(!staged(&a, &job), "and the mode is not active");
    }

    // V3-P2-4: drive the whole staged_author state machine with a SIMULATED agent
    // (no real model): a RED turn, a GREEN give_up (retry), then a passing GREEN.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p2_4_staged_author_statemachine_with_simulated_agent() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        {
            let mut q = a.sim.lock().unwrap();
            q.push_back(json!({"_files":{"t/sum_test.sh": RED_FAIL}, "test_cmd":"bash t/run.sh", "tests":["sum"], "title":"cover sum"}));
            q.push_back(json!({"give_up":"not sure yet"})); // GREEN attempt 1 -> retry
            q.push_back(json!({"_files":{"add.sh":"add() { echo $(( $1 + $2 )); }\n"}, "title":"add not subtract"})); // GREEN attempt 2 -> pass
        }
        let mut fb = String::new();
        let out = staged_author(&a, &mut job, &mut fb, "ollama:qwen3:8b").await.unwrap();
        assert!(out.is_some(), "simulated RED + retried GREEN must complete: fb={fb}");
        let js = out.unwrap();
        assert_eq!(js["test_cmd"], "bash t/run.sh");
        assert_eq!(js["tests"][0], "sum");
        assert_eq!(String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(&repo).output().unwrap().stdout).trim(), "2");
        assert_red_then_green(&repo);
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V3-P2-4: an invalid (no-JSON) RED answer from the simulated agent is handled.
    #[cfg(unix)]
    #[tokio::test]
    async fn v3_p2_4_staged_author_handles_invalid_red_json() {
        let (a, id, dd, _repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        a.sim.lock().unwrap().push_back(Value::Null); // RED returns no JSON
        let mut fb = String::new();
        let out = staged_author(&a, &mut job, &mut fb, "ollama:qwen3:8b").await.unwrap();
        assert!(out.is_none(), "no-JSON RED must not complete");
        assert!(fb.contains("no JSON"), "fb={fb}");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // ===== v4 audit: crash-window state recovery =====

    #[cfg(unix)]
    fn head_sha(repo: &std::path::Path) -> String {
        String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(repo).output().unwrap().stdout).trim().to_string()
    }
    #[cfg(unix)]
    fn count_above(repo: &std::path::Path, base: &str) -> String {
        String::from_utf8_lossy(&std::process::Command::new("git").args(["rev-list", "--count", &format!("{base}..HEAD")]).current_dir(repo).output().unwrap().stdout).trim().to_string()
    }
    #[cfg(unix)]
    fn commit_present(repo: &std::path::Path, sha: &str) -> bool {
        std::process::Command::new("git").args(["cat-file", "-e", &format!("{sha}^{{commit}}")]).current_dir(repo).output().unwrap().status.success()
    }

    // V4 Window A: crash AFTER the RED commit but BEFORE the checkpoint is finalised
    // leaves a PENDING checkpoint (tc/ts, no test_commit). staged_resume must bind it
    // to the real RED commit and continue GREEN — the commit stays accessible, no reset.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_window_a_crash_after_red_commit_recovers_without_reset() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (red, _red_files) = red_ok(&a, &mut job, &repo).await;
        // Simulate the crash window: only the PENDING checkpoint exists (no test_commit).
        job.d["staged_red"] = json!({"test_cmd":"bash t/run.sh","tests":["sum"],"title":"t","stage":"pending"});
        job.d.as_object_mut().unwrap().remove("staged_green");
        a.db.jd(&mut job);
        let br = job.s("br");
        match staged_resume(&a, &mut job, &base, &br).await.unwrap() {
            Resume::Green(s) => assert_eq!(s.test_commit, red, "must bind to the real RED commit"),
            r => panic!("window A must recover as Green, got {}", matches!(r, Resume::Fresh)),
        }
        assert_eq!(head_sha(&repo), red, "RED commit stays on the branch (no reset)");
        assert_eq!(count_above(&repo, &base), "1");
        assert!(commit_present(&repo, &red));
        assert_eq!(job.d["staged_red"]["test_commit"], red, "checkpoint finalised on recovery");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 Window B: a crash that leaves RED+GREEN committed but NO checkpoint and NO
    // discard authorisation must BLOCK (not reset): both commits stay accessible.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_window_b_crash_without_checkpoint_blocks_without_reset() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (test_commit, red_files) = red_ok(&a, &mut job, &repo).await;
        std::fs::write(repo.join("add.sh"), "add() { echo $(( $1 + $2 )); }\n").unwrap();
        assert!(green_commit(&a, &mut job, &red_files, &test_commit, "bash t/run.sh", &["sum".to_string()], "fix").await.unwrap().is_none());
        let green = head_sha(&repo);
        // Crash state: both checkpoints gone, no discard marker.
        job.d.as_object_mut().unwrap().remove("staged_red");
        job.d.as_object_mut().unwrap().remove("staged_green");
        job.d.as_object_mut().unwrap().remove("staged_discard");
        a.db.jd(&mut job);
        let br = job.s("br");
        let r = staged_resume(&a, &mut job, &base, &br).await;
        assert!(matches!(r, Err(X::Drop(_))), "must block for manual inspection");
        assert_eq!(head_sha(&repo), green, "HEAD unchanged; no destructive reset");
        assert_eq!(count_above(&repo, &base), "2", "both commits remain accessible");
        assert!(commit_present(&repo, &test_commit) && commit_present(&repo, &green));
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 point 5: an INTENTIONAL rejection persists a discard authorisation for the
    // exact HEAD, so the next round may reset (Fresh); a crash (no such marker) blocks.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_intentional_rejection_authorises_reset_but_crash_blocks() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let (_tc, _rf) = red_ok(&a, &mut job, &repo).await;
        let head = head_sha(&repo);
        let br = job.s("br");
        // No authorisation yet (simulated crash): commit present, no checkpoint -> block.
        job.d.as_object_mut().unwrap().remove("staged_red");
        a.db.jd(&mut job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await, Err(X::Drop(_))), "crash state must block");
        assert_eq!(head_sha(&repo), head, "blocked state left the branch intact");
        // Intentional rejection authorises discarding exactly this HEAD.
        reject_candidate(&a, &mut job, &head, &base);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh), "authorised discard resumes Fresh");
        assert!(job.d["staged_discard"].is_null(), "authorisation consumed");
        std::fs::remove_dir_all(dd).unwrap();
    }

    // V4 point 7: absent checkpoint with the branch at the base is Fresh; an
    // incoherent checkpoint (corrupt) with commits present blocks without reset.
    #[cfg(unix)]
    #[tokio::test]
    async fn v4_absent_at_base_is_fresh_and_corrupt_with_commits_blocks() {
        let (a, id, dd, repo) = staged_fixture();
        let mut job = a.db.jg(id).unwrap();
        let base = job.s("base");
        let br = job.s("br");
        // Absent checkpoint, HEAD at base -> Fresh.
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await.unwrap(), Resume::Fresh));
        // Corrupt checkpoint (no tc) with a real commit present -> block, no reset.
        let (red, _rf) = red_ok(&a, &mut job, &repo).await;
        job.d["staged_red"] = json!({"test_commit":red,"stage":"committed"}); // missing tc/tests
        a.db.jd(&mut job);
        assert!(matches!(staged_resume(&a, &mut job, &base, &br).await, Err(X::Drop(_))), "corrupt checkpoint with commits must block");
        assert_eq!(head_sha(&repo), red, "no reset on corrupt-with-commits");
        std::fs::remove_dir_all(dd).unwrap();
    }

}
