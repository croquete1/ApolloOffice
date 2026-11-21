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
    t.rsplit([':', '.', '/', '>', ' '])
        .find(|x| !x.trim().is_empty())
        .unwrap_or(t)
        .trim()
        .to_string()
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
    let safe_output = crate::hunt_health::safe_tail(output);
    let mut result = json!({
        "command":crate::hunt_health::safe_tail(command),
        "exit":exit,
        "output_tail":crate::sh::last(&safe_output, 4000),
        "candidate":candidate
    });
    if rerun { result["rerun"] = json!(true); }
    result
}

fn failure_details(stage: &str, host: &str, data: &Value, message: &str, at: i64) -> Value {
    let evidence = if stage == "ci" {
        data["ci_evidence"].clone()
    } else {
        Value::Null
    };
    json!({
        "stage":stage,
        "exit_code":evidence["exit"].as_i64(),
        "command":evidence["command"],
        "candidate":evidence["candidate"],
        "rerun":evidence["rerun"] == true,
        "host":host,
        "at":at,
        "message":crate::hunt_health::safe_tail(message),
        "output":crate::hunt_health::safe_tail(
            evidence["output_tail"].as_str().unwrap_or(message)
        )
    })
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
        "{h}\n{}:\n{}\n\n{} (from a previous agent): {}{}{}{}",
        j.orig(),
        wrap("issue", &format!("{}\n\n{}", j.title, j.s("body"))),
        if j.feat() || j.opt() { "Implementation plan" } else { "Repro result" },
        j.d["repro"],
        if fb.is_empty() { String::new() } else { format!("\n\nYour previous attempt was rejected by the pipeline:\n{fb}\nThe branch still contains your previous commits; fix the problems and make sure there are exactly two commits ({commit_roles}) on top of the base commit.") },
        rules(a, j),
        crate::quality::context(j)
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
            a.say("fixer", "resuming: RED+GREEN already committed; carrying it to verification without reset", Some(j.id));
            Ok(Resume::Finished(js))
        }
        // RED committed (possibly its finalise write was lost — window A): bind the
        // checkpoint to the real RED commit and continue GREEN.
        "1" => {
            let red = head.clone();
            let red_parent = ok(a, j, &format!("git rev-parse -q --verify {}^ 2>/dev/null || true", q(&red)), 60).await.unwrap_or_default();
            if red_parent != base {
                return drop(format!("staged resume: single commit not based on the base (HEAD={}); refusing to reset, manual inspection required", crate::sh::head(&head, 12)));
            }
            ok(a, j, &format!("git checkout -q -f -B {} {}", q(br), q(&red)), 300).await?;
            ok(a, j, &format!("git reset -q --hard {} && git clean -fdq", q(&red)), 300).await?;
            if bad_test_cmd(a, j, &tc).is_some() {
                return drop("staged resume: the frozen test command is no longer permitted; manual inspection required".to_string());
            }
            let (e, o) = run_wt(a, j, &tc).await;
            if e == 0 || Regex::new(CE).unwrap().is_match(&o) || !ts.iter().any(|t| o.contains(&bare(t))) {
                return drop(format!("staged resume: the RED commit {} no longer fails its frozen test legitimately; refusing to reset, manual inspection required", crate::sh::head(&red, 12)));
            }
            if red_files.is_empty() {
                red_files = ok(a, j, &format!("git diff --name-only {}..HEAD", q(base)), 60).await?.lines().map(|l| l.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            // Finalise the checkpoint now that the RED commit is bound.
            j.d["staged_red"] = json!({"test_commit": red, "test_cmd": tc, "tests": ts, "red_files": red_files, "title": title, "stage": "committed"});
            j.d["red_output"] = json!(crate::sh::last(&o, 2000));
            a.db.jd(j);
            a.say("fixer", "resuming from the preserved RED commit", Some(j.id));
            Ok(Resume::Green(RedState { tc, ts, red_files, test_commit: red, title }))
        }
        // Checkpoint exists but no commit was ever made (pending, crash before/at
        // commit). HEAD is the base, so dropping and rebuilding loses nothing.
        "0" => {
            drop_checkpoints(j);
            a.db.jd(j);
            Ok(Resume::Fresh)
        }
        _ => drop(format!("staged resume: unexpected branch shape ({head_count} commits above base); refusing to reset, manual inspection required")),
    }
}

/// One staged RED/GREEN author round. The office owns git history: it drives the
/// two model phases, checks each one and creates the test-only then fix-only commit
/// itself. A preserved RED checkpoint is resumed rather than rebuilt. On a
/// controlled rejection it returns Ok(None) with `fb` set so the caller retries.
async fn staged_author(a: &Ax, j: &mut Job, fb: &mut String, md: &str) -> R<Option<Value>> {
    let base = j.s("base");
    let br = j.s("br");
    use_(a, j, md);
    j.d["author"] = json!(md);
    a.db.jd(j);

    let RedState { tc, ts, red_files, test_commit, title: red_title } = match staged_resume(a, j, &base, &br).await? {
        Resume::Finished(js) => return Ok(Some(js)),
        Resume::Green(s) => {
            a.say("fixer", &format!("resuming from the preserved RED commit: {}", s.ts.join(", ")), Some(j.id));
            s
        }
        Resume::Fresh => {
            // Fresh RED. `-f` discards any dirty tracked file left by a previous
            // round; the hard reset + clean restore a pristine base tree.
            ok(a, j, &format!("git checkout -q -f -B {} {}", q(&br), q(&base)), 300).await?;
            ok(a, j, &format!("git reset -q --hard {} && git clean -fdq", q(&base)), 300).await?;
            a.st("fixer", "work", Some(j.id), "");
            a.say("fixer", &format!("RED: writing the failing test for {}", j.tag()), Some(j.id));
            let red = match ask_phase(a, j, md, "sred", staged_prompt(a, j, fb, true, "")).await {
                Ok(v) => v,
                Err(X::Back(e)) => return Err(X::Back(e)),
                Err(X::Drop(e)) => return Err(X::Drop(e)),
                Err(e) => { *fb = format!("RED attempt failed: {e}. Inspect the tree and write a failing test."); return Ok(None); }
            };
            a.st("fixer", "idle", None, "");
            if red.is_null() { *fb = "The RED phase returned no JSON.".into(); return Ok(None); }
            if let Some(g) = red["give_up"].as_str() { *fb = format!("Author gave up in the RED phase: {g}."); return Ok(None); }
            let tc = red["test_cmd"].as_str().unwrap_or("").to_string();
            let ts: Vec<String> = red["tests"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).filter(|x| !x.is_empty()).collect();
            if tc.is_empty() || ts.is_empty() { *fb = "The RED answer is missing test_cmd or tests.".into(); return Ok(None); }
            let red_title = red["title"].as_str().unwrap_or("").to_string();
            // Persist the RED identity BEFORE creating the commit (window A): if the
            // process dies between `git commit` and finalising this checkpoint, the
            // pending checkpoint still carries tc/ts so staged_resume can bind it to
            // the real RED commit rather than discard it.
            j.d["staged_red"] = json!({"test_cmd": tc, "tests": ts, "title": red_title, "stage": "pending"});
            a.db.jd(j);
            if let Some(m) = red_commit(a, j, &tc, &ts, &red_title).await? {
                // No commit was made; HEAD is still the base, so dropping is safe.
                drop_checkpoints(j);
                a.db.jd(j);
                *fb = m;
                return Ok(None);
            }
            let test_commit = ok(a, j, "git rev-parse HEAD", 60).await?;
            let red_files: Vec<String> = ok(a, j, &format!("git diff --name-only {}..HEAD", q(&base)), 60).await?.lines().map(|l| l.trim().to_string()).filter(|s| !s.is_empty()).collect();
            // Finalise: freeze command, names, the exact commit and files. GREEN
            // cannot change them and the verifier uses only these (B6 / A6 / P1-3).
            j.d["staged_red"] = json!({"test_commit": test_commit, "test_cmd": tc, "tests": ts, "red_files": red_files, "title": red_title, "stage": "committed"});
            a.db.jd(j);
            a.say("fixer", &format!("RED committed (test fails): {}", ts.join(", ")), Some(j.id));
            RedState { tc, ts, red_files, test_commit, title: red_title }
        }
    };

    // GREEN phase: the fix, with the RED test commit preserved. Retry GREEN a few
    // times WITHOUT discarding the RED commit — only the working tree is reset to
    // the test commit between attempts, so RED work survives a failed GREEN (A1).
    for attempt in 0..2u32 {
        ok(a, j, &format!("git checkout -q -f -B {} {}", q(&br), q(&test_commit)), 300).await?;
        ok(a, j, &format!("git reset -q --hard {} && git clean -fdq", q(&test_commit)), 300).await?;
        a.st("fixer", "work", Some(j.id), "");
        a.say("fixer", &format!("GREEN{}: implementing the fix for {}", if attempt > 0 { format!(" (retry {attempt})") } else { String::new() }, j.tag()), Some(j.id));
        let green = match ask_phase(a, j, md, "sgreen", staged_prompt(a, j, fb, false, &tc)).await {
            Ok(v) => v,
            Err(X::Back(e)) => return Err(X::Back(e)),
            Err(X::Drop(e)) => return Err(X::Drop(e)),
            Err(e) => { *fb = format!("GREEN attempt failed: {e}. The failing test is committed; implement the fix."); continue; }
        };
        a.st("fixer", "idle", None, "");
        if green.is_null() { *fb = "The GREEN phase returned no JSON.".into(); continue; }
        if let Some(g) = green["give_up"].as_str() { *fb = format!("Author gave up in the GREEN phase: {g}."); continue; }
        // Freeze identity: the proof is the RED test command and names, never the
        // GREEN answer's (B6). GREEN supplies only the human-facing fields.
        let mut js = green.clone();
        js["test_cmd"] = json!(tc);
        js["tests"] = json!(ts);
        if js["title"].as_str().unwrap_or("").trim().is_empty() { js["title"] = json!(red_title); }
        if let Some(m) = green_commit(a, j, &red_files, &test_commit, &tc, &ts, js["title"].as_str().unwrap_or("fix")).await? { *fb = m; continue; }
        // Both commits exist. Persist the GREEN result alongside the RED checkpoint
        // (transactional handoff): a crash before the caller adjudicates this pair
        // is recovered by staged_resume WITHOUT discarding the commit (V3-P1-2). The
        // caller clears both once it has accepted or rejected the pair.
        let green_commit_sha = ok(a, j, "git rev-parse HEAD", 60).await?;
        j.d["staged_green"] = json!({"commit": green_commit_sha, "js": js});
        a.db.jd(j);
        a.say("fixer", "GREEN committed (test passes)", Some(j.id));
        return Ok(Some(js));
    }
    // RED is preserved on the branch and in the checkpoint; the outer loop decides
    // the next round, which will resume from it rather than rebuild it.
    Ok(None)
}

fn fix_attempt_start(a: &Ax, j: &Job, ordinal: u64, md: &str) -> (i64, i64) {
    let started = crate::db::now();
    a.db.ch(
        "update fix_attempts set finished=?,elapsed_ms=?-started,outcome='interrupted',gate='model' where job=? and finished is null",
        pa![started, started, j.id],
    );
    let next = a.db.n(
        "select coalesce(max(attempt),0)+1 n from fix_attempts where job=?",
        pa![j.id],
    );
    let attempt = next.max(ordinal.min(i64::MAX as u64) as i64);
    let id = a.db.run(
        "insert into fix_attempts(job,attempt,model,started) values(?,?,?,?)",
        pa![j.id, attempt, md.to_string(), started],
    );
    (id, started)
}

fn fix_attempt_finish(a: &Ax, id: i64, started: i64, outcome: &str, gate: &str) {
    if id < 0 {
        return;
    }
    let finished = crate::db::now();
    a.db.ch(
        "update fix_attempts set finished=?,elapsed_ms=?,outcome=?,gate=? where id=?",
        pa![
            finished,
            (finished - started).max(0),
            outcome.to_string(),
            gate.to_string(),
            id
        ],
    );
}

fn fix_attempt_finish_active(a: &Ax, j: &Job, outcome: &str, gate: &str) {
    if let Some(attempt) = a.db.one(
        "select id,started from fix_attempts where job=? and finished is null order by attempt desc limit 1",
        pa![j.id],
    ) {
        if let (Some(id), Some(started)) = (attempt["id"].as_i64(), attempt["started"].as_i64()) {
            fix_attempt_finish(a, id, started, outcome, gate);
        }
    }
}

async fn fix(a: &Ax, j: &mut Job, fb0: String, k0: u64) -> R<Value> {
    if staged_conflict(a, j) {
        return drop("staged TDD pilot requires free_first OFF: free-first routing would replace the configured FIXER/REVIEWER. Disable free_first for this installation or remove the repo from staged_repos.".to_string());
    }
    crate::quality::prepare(a, j).await?;
    let mut fb = fb0;
    let st = staged(a, j);
    let route_on = crate::routing::enabled(a);
    let plan = crate::routing::plan(a);
    let rounds = if st || !route_on { RND + 1 } else { plan.len() as u64 };
    let primary = a.md("fixer");
    let complex_model = a.md("big");
    // Keep the low-cost path for ordinary work, but hand a failed local-model
    // candidate to the configured Codex model for independent repair. Free-first
    // routing retains its own bounded plan, and complex jobs start on `big`.
    let fallback = !route_on
        && a.set()["big"].as_bool().unwrap_or(true)
        && primary.starts_with("ollama:")
        && complex_model.starts_with("codex:")
        && primary != complex_model;
    for k in k0..rounds {
        kill(a, j)?;
        a.db.jset(j.id, &[("st", json!("fix"))]);
        a.st("fixer", "work", Some(j.id), "");
        a.say(
            "fixer",
            &format!(
                "{}fixing {}",
                if k > 0 {
                    format!("round {}: ", k + 1)
                } else {
                    String::new()
                },
                j.tag()
            ),
            Some(j.id),
        );
        let c = cx(a, j);
        let md = if st {
            match staged_fallback(a, j) {
                Some(fallback_model) if k + 1 == rounds && k > 0 => {
                    a.say("fixer", &format!("primary FIXER exhausted after {k} round(s); one fallback attempt with {fallback_model}, continuing from the preserved RED commit if present"), Some(j.id));
                    fallback_model
                }
                _ => a.md("fixer"),
            }
        } else if route_on {
            plan[k as usize].clone()
        } else if c || (fallback && k > 0) {
            complex_model.clone()
        } else {
            primary.clone()
        };
        let (attempt_id, attempt_started) = fix_attempt_start(a, j, k + 1, &md);
        crate::routing::event(a, Some(j.id), "fixer", k as usize, &md, &fb);
        let js = if st {
            match staged_author(a, j, &mut fb, &md).await {
                Ok(Some(value)) => value,
                Ok(None) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "staged");
                    a.st("fixer", "idle", None, "sad");
                    continue;
                }
                Err(X::Back(e)) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "interrupted", "model");
                    return Err(X::Back(e));
                }
                Err(X::Drop(e)) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "staged");
                    return Err(X::Drop(e));
                }
                Err(e) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "staged");
                    return Err(e);
                }
            }
        } else {
            use_(a, j, &md);
            j.d["author"] = json!(md);
            let mut o = Ask::new("fixer", md, P(if j.opt() { "optimizer" } else { "fixer" }), fixp(a, j, &fb));
            o.job = Some(j.id);
            o.rn = rn(a, j);
            o.turns = if c { 200 } else { 120 };
            o.t = if c { 180 * 60 } else { 120 * 60 };
            let (_, value) = match ask(a, o).await {
                Ok(v) => v,
                Err(X::Back(e)) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "interrupted", "model");
                    return Err(X::Back(e));
                }
                Err(X::Drop(e)) if fallback => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "model");
                    fb = format!("Model attempt failed: {e}. Inspect the existing tree and preserve useful work.");
                    continue;
                }
                Err(X::Drop(e)) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "model");
                    return Err(X::Drop(e));
                }
                Err(e) if route_on || fallback => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "model");
                    fb = format!("Model attempt failed: {e}. Inspect the existing tree and preserve useful work.");
                    continue;
                }
                Err(e) => {
                    fix_attempt_finish(a, attempt_id, attempt_started, "failed", "model");
                    return Err(e);
                }
            };
            a.st("fixer", "idle", None, "");
            value
        };
        if js.is_null() {
            fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "output");
            fb = "Your answer contained no JSON.".into();
            continue;
        }
        if let Some(g) = js["give_up"].as_str() {
            fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "output");
            a.st("fixer", "idle", None, "sad");
            if route_on || fallback {
                fb = format!(
                    "Previous model gave up: {g}. Inspect the tree and repair the actual issue."
                );
                continue;
            }
            return drop(format!("Fixer gave up: {g}"));
        }
        a.say(
            "fixer",
            &format!("done: {}", js["title"].as_str().unwrap_or("")),
            Some(j.id),
        );
        j.d["f0"] = js.clone();
        j.d["k"] = json!(k);
        a.db.jd(j);
        a.flow("fixer", "ci", "proof");
        let verification = if j.kind == "review" {
            vrev(a, j, &js).await
        } else {
            verify(a, j, &js).await
        };
        fb = match verification {
            Ok(value) => value,
            Err(e) => {
                fix_attempt_finish(a, attempt_id, attempt_started, "failed", "verification");
                return Err(e);
            }
        };
        if !fb.is_empty() {
            fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "verification");
            a.say(
                "fixer",
                &format!("proof missing: {}", fb.lines().next().unwrap_or("")),
                Some(j.id),
            );
            if route_on {
                j.d["verification_feedback"] = json!(fb.clone());
                let diagnosis = diagnosis(a, j, &mut js.clone()).await?;
                fb.push_str(&format!("\n{diagnosis}"));
            }
            if st {
                let head = ok(a, j, "git rev-parse HEAD", 60).await?;
                let base = j.s("base");
                reject_candidate(a, j, &head, &base);
            }
            continue;
        }
        fb = match ci(a, j).await {
            Ok(value) => value,
            Err(e) => {
                fix_attempt_finish(a, attempt_id, attempt_started, "failed", "ci");
                return Err(e);
            }
        };
        if fb.starts_with("CI aborted after") {
            fix_attempt_finish(a, attempt_id, attempt_started, "blocked", "ci");
            a.st("fixer", "idle", None, "sad");
            if st {
                let head = ok(a, j, "git rev-parse HEAD", 60).await?;
                let base = j.s("base");
                reject_candidate(a, j, &head, &base);
            }
            return drop(format!("{fb} on {}", j.h()));
        }
        if !fb.is_empty() {
            fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "ci");
            if route_on {
                j.d["verification_feedback"] = json!(fb.clone());
                let diagnosis = diagnosis(a, j, &mut js.clone()).await?;
                fb.push_str(&format!("\n{diagnosis}"));
            }
            if st {
                let head = ok(a, j, "git rev-parse HEAD", 60).await?;
                let base = j.s("base");
                reject_candidate(a, j, &head, &base);
            }
            continue;
        }
        j.d["verification_feedback"] = json!("Executed regression proof and repository CI passed.");
        let mut f = js.clone();
        fb = match review(a, j, &mut f).await {
            Ok(value) => value,
            Err(e) => {
                fix_attempt_finish(a, attempt_id, attempt_started, "failed", "review");
                return Err(e);
            }
        };
        if !fb.is_empty() {
            fix_attempt_finish(a, attempt_id, attempt_started, "rejected", "review");
            a.flow("reviewer", "fixer", "back");
            if st {
                let head = ok(a, j, "git rev-parse HEAD", 60).await?;
                let base = j.s("base");
                reject_candidate(a, j, &head, &base);
            }
            continue;
        }
        if st {
            drop_checkpoints(j);
        }
        fix_attempt_finish(a, attempt_id, attempt_started, "accepted", "");
        j.d["f"] = f.clone();
        a.db.jd(j);
        return Ok(f);
    }
    a.st("fixer", "idle", None, "sad");
    drop(format!(
        "no valid fix after {} rounds: {}",
        rounds,
        fb.lines().next().unwrap_or("")
    ))
}

async fn reb(a: &Ax, j: &mut Job) -> R<()> {
    a.db.jset(j.id, &[("st", json!("fix"))]);
    a.st("fixer", "work", Some(j.id), "");
    let mn = j.s("mn");
    a.say(
        "fixer",
        &format!("rebasing {}#{} onto {mn}", j.repo, j.pr.unwrap_or(0)),
        Some(j.id),
    );
    pull(
        a,
        j,
        &format!("{}{}.git", gitb(), j.repo),
        &mn,
        &format!("refs/remotes/origin/{mn}"),
    )
    .await?;
    let r = x(a, j, &format!("git rebase -q origin/{}", q(&mn)), 600).await;
    if r.c != 0 {
        let fs = x(a, j, "git diff --name-only --diff-filter=U", 60)
            .await
            .o
            .trim()
            .to_string();
        a.say(
            "fixer",
            &format!("conflicts in {} file(s), resolving", fs.lines().count()),
            Some(j.id),
        );
        let md = a.md("fixer");
        use_(a, j, &md);
        let mut o = Ask::new("fixer", md, P("rebase"), format!("Repository {}. The rebase of pull request branch {} (PR #{}: {}) onto origin/{mn} stopped with conflicts in:\n{fs}\n\nResolve every conflict, `git add` the files and run `GIT_EDITOR=true git rebase --continue`; repeat until the rebase is finished. Do not add other changes. Run targeted builds and tests for touched files. Finish with a clean tree: `git status --porcelain` must print nothing. Then answer only with JSON {{\"title\": \"one line on how you resolved it\"}} or {{\"give_up\": \"reason\"}}.", j.repo, j.s("br"), j.pr.unwrap_or(0), wrap("title", &j.title)));
        o.job = Some(j.id);
        o.rn = rn(a, j);
        o.turns = 80;
        o.t = 90 * 60;
        let (_, js) = ask(a, o).await?;
        if js.is_null() || js["give_up"].is_string() {
            x(a, j, "git rebase --abort", 60).await;
            return drop(format!(
                "conflict not resolved: {}",
                js["give_up"].as_str().unwrap_or("no answer")
            ));
        }
        if x(
            a,
            j,
            "test -d .git/rebase-merge -o -d .git/rebase-apply",
            60,
        )
        .await
        .c == 0
        {
            x(a, j, "git rebase --abort", 60).await;
            return drop("rebase left unfinished");
        }
        j.d["fx"] = json!(js["title"].as_str().unwrap_or("resolved conflicts"));
    }
    if x(
        a,
        j,
        &format!("git merge-base --is-ancestor origin/{} HEAD", q(&mn)),
        60,
    )
    .await
    .c != 0
    {
        return drop("branch is not on top of the default branch after rebase");
    }
    let dt = ok(a, j, "git status --porcelain", 60).await?;
    if !dt.is_empty() {
        return drop(format!(
            "working tree not clean after rebase: {}",
            dt.lines().take(5).collect::<Vec<_>>().join(", ")
        ));
    }
    j.d["base"] = json!(ok(a, j, &format!("git rev-parse origin/{}", q(&mn)), 60).await?);
    let fx = j.s("fx");
    j.d["f"] = json!({"title": if fx.is_empty() { format!("Rebase onto {mn}") } else { format!("Rebase onto {mn}: {fx}") }, "tests": [], "test_cmd": j.s("test_cmd"), "cause": "", "fix": ""});
    a.db.jd(j);
    let tc = j.s("test_cmd");
    if !tc.is_empty() && !Regex::new(crate::mcp::BAD).unwrap().is_match(&tc) {
        let (e, _) = run_t(a, j, "HEAD", &tc, &[]).await;
        ok(a, j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
        if e != 0 {
            return drop("PR tests fail after rebase");
        }
    }
    a.st("fixer", "idle", None, "");
    let c = ci(a, j).await?;
    if !c.is_empty() {
        return drop(rbw(j, &c));
    }
    if !fx.is_empty() {
        let df = ok(a, j, &format!("git range-diff {o}~$(git rev-list --count {b}..HEAD)..{o} {b}..HEAD | head -c 40000", o = j.s("old"), b = j.s("base")), 120).await.unwrap_or_default();
        let fd = ok(
            a,
            j,
            &format!("git diff {}..HEAD | head -c 30000", j.s("base")),
            120,
        )
        .await
        .unwrap_or_default();
        let md = a.md("reviewer");
        use_(a, j, &md);
        let mut o = Ask::new("reviewer", md, P("rebrev"), format!("PR #{} {}\nResolution note: {fx}\n\ngit range-diff (old PR commits vs rebased commits):\n{df}\n\nFinal diff {mn}..HEAD:\n{fd}", j.pr.unwrap_or(0), wrap("title", &j.title)));
        o.job = Some(j.id);
        o.ef = Some("medium");
        let (_, js) = ask(a, o).await?;
        let g = js["ok"].as_bool().unwrap_or(false);
        a.st("reviewer", "idle", None, if g { "ok" } else { "sad" });
        if !g {
            return drop(format!(
                "conflict resolution rejected by Reviewer: {}",
                crate::sh::head(&js["issues"].to_string(), 200)
            ));
        }
    }
    Ok(())
}

fn rbw(j: &Job, c: &str) -> String {
    if c.starts_with("CI aborted after") {
        format!("after rebase: {c} on {}", j.h())
    } else {
        format!(
            "CI red after rebase: {}",
            crate::sh::head(c.lines().next().unwrap_or(""), 160)
        )
    }
}

async fn taken(a: &Ax, j: &Job) -> R<String> {
    if j.kind != "fix" || j.own() || j.num == 0 {
        return Ok(String::new());
    }
    let me = crate::gh::login(a).await;
    let is = ghj(
        &[
            "issue",
            "view",
            &j.num.to_string(),
            "-R",
            &j.repo,
            "--json",
            "state,assignees,comments",
        ],
        60,
    )
    .await
    .map_err(X::E)?;
    if is["state"] != "OPEN" {
        return Ok("someone was faster: issue is closed".into());
    }
    let oa: Vec<String> = is["assignees"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["login"].as_str().map(String::from))
        .filter(|x| *x != me)
        .collect();
    if !oa.is_empty() {
        return Ok(format!("someone was faster: assigned to {}", oa.join(", ")));
    }
    let cl = Regex::new(r"(?i)\b(i'?m (now )?(working|taking|looking into|on it|fixing)|i am (now )?(working|taking|fixing)|i'?ll (take|fix|work|handle|open a pr|send a pr)|i will (take|fix|work|handle)|working on (this|it|a fix)|claim(ed|ing)? (this|it|#[0-9]+)|assign (this |it )?(to )?me|pr (is )?(coming|incoming|soon|on the way)|on it\b)").unwrap();
    let va = j.d["va"].as_i64().unwrap_or(j.t0);
    for c in is["comments"].as_array().cloned().unwrap_or_default() {
        let by = c["author"]["login"].as_str().unwrap_or("");
        let t = chrono::DateTime::parse_from_rfc3339(c["createdAt"].as_str().unwrap_or(""))
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        if by != me && t > va && cl.is_match(c["body"].as_str().unwrap_or("")) {
            return Ok(format!("someone was faster: {by} claimed it in a comment"));
        }
    }
    let lv: Vec<String> = crate::gh::live(crate::gh::refs(&j.repo, j.num).await.map_err(X::E)?)
        .into_iter()
        .filter(|p| p.by != me)
        .map(|p| format!("{}#{}", p.rp, p.n))
        .collect();
    if !lv.is_empty() {
        return Ok(format!("someone was faster: {}", lv.join(", ")));
    }
    Ok(String::new())
}

async fn race(a: &Ax, j: &Job, at: &str) -> R<()> {
    let t = taken(a, j).await?;
    if !t.is_empty() {
        return drop(format!("{t} ({at})"));
    }
    Ok(())
}

async fn gate(a: &Ax, j: &mut Job) -> R<()> {
    a.db.jset(j.id, &[("st", json!("gate"))]);
    a.st("submit", "work", Some(j.id), "");
    a.say(
        "submit",
        &format!("pre-push gate for {}", j.tag()),
        Some(j.id),
    );
    need(a, "submit", &["gh", &j.h()]).await?;
    if j.kind != "fix" {
        let p = ghj(
            &[
                "pr",
                "view",
                &j.pr.unwrap_or(0).to_string(),
                "-R",
                &j.repo,
                "--json",
                "state",
            ],
            60,
        )
        .await
        .map_err(X::E)?;
        if p["state"] != "OPEN" {
            return drop("PR is no longer open");
        }
        return Ok(());
    }
    race(a, j, "at the gate").await?;
    let mn = j.s("mn");
    pull(
        a,
        j,
        &format!("{}{}.git", gitb(), j.repo),
        &mn,
        &format!("refs/remotes/origin/{mn}"),
    )
    .await?;
    let nb = ok(a, j, &format!("git rev-parse origin/{}", q(&mn)), 60).await?;
    if !j.own() && j.num > 0 {
        let hit = ok(
            a,
            j,
            &format!(
                "git log --format='%h %s' {}..origin/{} | grep -E '#{}([^0-9]|$)' || true",
                j.s("base"),
                q(&mn),
                j.num
            ),
            60,
        )
        .await?;
        if !hit.is_empty() {
            return drop(format!(
                "someone was faster: commit on {mn} mentions #{}: {}",
                j.num,
                hit.lines().next().unwrap_or("")
            ));
        }
    }
    if nb == j.s("base") {
        let result = crate::quality::check(a, j).await?;
        if !result.is_empty() {
            return drop(result);
        }
        a.say(
            "submit",
            "default branch unchanged, proofs still valid",
            Some(j.id),
        );
        return Ok(());
    }
    a.say(
        "submit",
        "default branch moved, rebasing and re-verifying everything",
        Some(j.id),
    );
    let rb = x(a, j, &format!("git rebase -q origin/{}", q(&mn)), 600).await;
    if rb.c != 0 {
        x(a, j, "git rebase --abort", 60).await;
        return drop("rebase conflict with the current default branch");
    }
    let ob = j.s("base");
    j.d["base"] = json!(nb);
    a.db.jd(j);
    let f = j.d["f"].clone();
    let v = verify(a, j, &f).await?;
    if !v.is_empty() {
        j.d["base"] = json!(ob);
        return drop(format!("after rebase: {}", v.lines().next().unwrap_or("")));
    }
    let c = ci(a, j).await?;
    if !c.is_empty() {
        return drop(rbw(j, &c));
    }
    Ok(())
}

async fn tmpl(r: &str) -> Option<String> {
    for p in [
        ".github/pull_request_template.md",
        ".github/PULL_REQUEST_TEMPLATE.md",
        "pull_request_template.md",
        "PULL_REQUEST_TEMPLATE.md",
        "docs/pull_request_template.md",
    ] {
        if let Some(t) = crate::gh::file(r, p).await {
            if !t.trim().is_empty() {
                return Some(t);
            }
        }
    }
    None
}

async fn mkbody(a: &Ax, j: &mut Job) -> R<()> {
    let f = j.d["f"].clone();
    let ci = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
    let ft = fo(a, j);
    let mut b = body_lang(j, &f, &ci, ft, pr_pt(a, j));
    if j.kind == "fix" {
        if let Some(t) = tmpl(&j.repo).await {
            a.say(
                "submit",
                "repo has a PR template, filling it in",
                Some(j.id),
            );
            let md = a.md("reviewer");
            let instructions = if pr_pt(a, j) {
                "Preencha o modelo de Pull Request com a informação abaixo. Preserve a estrutura e os títulos em PT-PT. Todo o texto narrativo deve estar em português de Portugal, incluindo títulos e listas. Assinale uma checkbox apenas quando os factos provarem o respetivo resultado. Remova instruções e comentários HTML do modelo. Nunca invente testes ou aprovações. Preserve identificadores técnicos, comandos e a referência 'Fixes #N'. Responda apenas com JSON de chave 'body' e valor Markdown."
            } else {
                "Fill in this repository's pull request template for the change below. Keep the template's headings and checklist structure. Tick a checkbox only when the facts below prove it (tests added, tests pass, CI command passes, linked issue). Remove instructions and HTML comments that are meant for the author. Put the facts into the matching sections; if a fact has no section, keep it in a section at the end. Use only the facts given, never invent. Answer only with JSON with key 'body' and Markdown string value."
            };
            let mut o = Ask::new(
                "submit",
                md,
                P("desc"),
                format!(
                    "{instructions}\n\n{}\n\nFacts (our PR text):\n{}",
                    wrap("template", &crate::sh::head(&t, 8000)),
                    body_lang(j, &f, &ci, "", pr_pt(a, j))
                ),
            );
            o.job = Some(j.id);
            o.ef = Some("low");
            if let Ok((_, js)) = ask(a, o).await {
                if let Some(s) = js["body"].as_str().filter(|s| s.len() > 40) {
                    // A template model must not silently reintroduce English headings.
                    let wrong_language = pr_pt(a, j) && english_pr_heading(s);
                    let missing_reference = j.kind == "fix"
                        && !j.own()
                        && !s.contains(&format!(
                            "{} #{}",
                            if j.feat() { "Closes" } else { "Fixes" },
                            j.num
                        ));
                    if wrong_language || missing_reference {
                        a.say("submit", "modelo de PR rejeitado por idioma ou referência em falta; usado o corpo estruturado", Some(j.id));
                    } else {
                        b = s.trim().to_string();
                        if j.own() {
                            b = san(&b);
                        }
                        if !b.contains(ft) {
                            b = format!("{b}\n\n{ft}");
                        }
                    }
                }
            }
            a.st("submit", "work", Some(j.id), "");
        }
    }
    b = ensure_issue_closure(j, b);
    j.d["pt"] = json!(f["title"].as_str().unwrap_or(&j.title));
    j.d["pb"] = json!(b);
    j.d["df"] = json!(ok(
        a,
        j,
        &format!("git diff {}..HEAD | head -c 120000", j.s("base")),
        120
    )
    .await
    .unwrap_or_default());
    a.db.jd(j);
    Ok(())
}

fn auto_merge_requested(prior: Option<&Value>, newly_created: bool, j: &Job, repo_enabled: bool) -> bool {
    if let Some(prior) = prior {
        return prior["auto_merge"].as_bool() == Some(true);
    }
    newly_created && j.kind == "fix" && !j.feat() && !j.opt() && repo_enabled
}


async fn submit(a: &Ax, j: &mut Job) -> R<(String, i64)> {
    // Every path, including recovery/resume, must pass this boundary before any
    // squash, push or GitHub request. A resumed job is not an approval grant.
    if crate::routing::enabled(a) && j.d["ap"].as_bool() != Some(true) {
        // Persist the hold before any fallible worker/preview operation.
        a.db.jset(
            j.id,
            &[
                ("st", json!("approve")),
                (
                    "why",
                    json!("waiting for explicit owner approval, including resumed jobs"),
                ),
            ],
        );
        let f = j.d["f"].clone();
        let ci = rp(a, &j.repo)["ci"].as_str().unwrap_or("").to_string();
        let title = pr_title(j.d["pt"].as_str(), &f, &j.title)?;
        j.d["pt"] = json!(title);
        let footer = fo(a, j);
        j.d["pb_footer"] = json!(footer);
        j.d["pb"] = json!(body_lang(j, &f, &ci, footer, pr_pt(a, j)));
        let tip = if j.s("br").is_empty() {
            "HEAD".to_string()
        } else {
            j.s("br")
        };
        let range = q(&format!("{}..{}", j.s("base"), tip));
        j.d["df"] = json!(
            ok(a, j, &format!("git diff {range} -- | head -c 120000"), 120)
                .await
                .unwrap_or_default()
        );
        a.db.jd(j);
        a.say(
            "submit",
            "submission held: resumed work does not grant owner approval",
            Some(j.id),
        );
        clean(a, j).await;
        return Err(X::Back(String::new()));
    }
    a.db.jset(j.id, &[("st", json!("submit"))]);
    a.st("submit", "work", Some(j.id), "");
    a.flow("reviewer", "submit", &j.tag());
    need(a, "submit", &["gh", &j.h()]).await?;
    let f = j.d["f"].clone();
    let ft = stored_pr_footer(j, fo(a, j));
    let tr = trs(j);
    let ti = pr_title(j.d["pt"].as_str(), &f, &j.title)?;
    if j.kind == "fix" && !j.d["sq"].as_bool().unwrap_or(false) {
        let rf = if j.ext() { "Reported to us directly".to_string() } else if j.hunt() || j.num == 0 { format!("Found while testing {}", j.wh()) } else if j.opt() { format!("Closes #{}", j.num) } else { format!("Fixes #{}", j.num) };
        let subject = if j.opt() {
            optimization_commit_subject(&ti)?
        } else {
            commit_subject(&ti, j.feat())?
        };
        ok(a, j, &format!("git reset -q --soft {} && git commit -q -F - <<'MSG'\n{subject}\n\n{rf}\n\n{}\nMSG", j.s("base"), tr.join("\n")), 120).await?;
        j.d["sq"] = json!(true);
        a.db.jd(j);
    }
    if (j.kind == "review" || j.kind == "rebase" && !j.s("fx").is_empty())
        && !j.d["sq"].as_bool().unwrap_or(false)
        && !tr.is_empty()
    {
        ok(
            a,
            j,
            &format!(
                "git commit -q --amend --no-edit {}",
                tr.iter()
                    .map(|t| format!("--trailer {}", q(t)))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            120,
        )
        .await?;
        j.d["sq"] = json!(true);
        a.db.jd(j);
    }
    let sha = ok(a, j, "git rev-parse HEAD", 60).await?;
    // Prova de publicação: árvore da PR final e commit explícitos.
    // É necessário que o CI tenha executado nesta mesma árvore.
    let publish_tree = ok(a, j, "git rev-parse HEAD^{tree}", 60).await?;
    j.d["publish_tree"] = json!(publish_tree);
    a.db.jd(j);
    let pb = j.d["pb"].as_str().map(String::from).unwrap_or_else(|| {
        body_lang(
            j,
            &f,
            rp(a, &j.repo)["ci"].as_str().unwrap_or(""),
            ft.as_str(),
            pr_pt(a, j),
        )
    });
    let msg = ok(a, j, "git log -1 --format=%B", 60).await?;
    if Regex::new(r"#0\b").unwrap().is_match(&pb) || Regex::new(r"#0\b").unwrap().is_match(&msg) {
        return drop("PR text or commit still references #0, not pushing");
    }
    if (j.kind != "rebase" || !j.s("fx").is_empty())
        && (!tr.iter().all(|t| msg.contains(t)) || j.kind == "fix" && !pb.contains(ft.as_str()))
    {
        return drop("AI disclosure missing (Co-Authored-By trailer or footer), not pushing");
    }
    if std::env::var("APOLLO_OFFICE_DRY")
        .map(|v| v == "1")
        .unwrap_or(false)
        || j.d["dry"].as_bool().unwrap_or(false)
    {
        a.say(
            "submit",
            &format!(
                "DRY: would push {} ({}) and open PR \"{ti}\"",
                j.s("br"),
                crate::sh::head(&sha, 8)
            ),
            Some(j.id),
        );
        return Ok((String::new(), 0));
    }
    let me = crate::gh::login(a).await;
    let mt = crate::gh::meta(a, &j.repo).await.map_err(X::E)?;
    let hr = if j.kind == "fix" {
        if mt["push"].as_bool().unwrap_or(false) {
            j.repo.clone()
        } else {
            let fk = ghj(
                &[
                    "api",
                    "-X",
                    "POST",
                    &format!("repos/{}/forks", j.repo),
                    "-f",
                    "default_branch_only=true",
                ],
                120,
            )
            .await
            .map_err(X::E)?;
            let n = fk["full_name"].as_str().unwrap_or("").to_string();
            if n.is_empty() {
                return e("could not create a fork");
            }
            for _ in 0..12 {
                if ghj(&["api", &format!("repos/{n}")], 30).await.is_ok() {
                    break;
                }
                sl(5000).await;
            }
            n
        }
    } else {
        j.s("head")
    };
    let url = format!("{}{}.git", gitb(), hr);
    let br = j.s("br");
    let pr = format!("refs/ao/push/{}", j.id);
    let _ = lg(
        a,
        &j.repo,
        &[
            "fetch",
            "-q",
            "--no-tags",
            &format!("{}{}.git", gitb(), j.repo),
            &format!("+refs/heads/{}:refs/ao/main", j.s("mn")),
        ],
        true,
        1800,
    )
    .await;
    let tf = format!("/tmp/ao-out-{}.bundle", j.id);
    let bf = a.dd.join("run").join(format!("out-{}.bundle", j.id));
    let _ = std::fs::create_dir_all(bf.parent().unwrap());
    let bfs = bf.display().to_string();
    let mut got = false;
    for ex in [true, false] {
        let c = format!("cd \"{}\" && rm -f {tf} && git bundle create -q {tf} {} {} >/dev/null 2>&1 || exit 3; base64 < {tf} | tr -d '\\n\\r'; rm -f {tf}", dir(a, j), q(&format!("refs/heads/{br}")), if ex { format!("^{}", q(&j.s("base"))) } else { String::new() });
        let o = crate::sh::hxk(a, &j.h(), &c, 1800).await;
        if o.c != 0 {
            continue;
        }
        use base64::Engine;
        let Ok(b) = base64::engine::general_purpose::STANDARD.decode(o.o.trim()) else {
            continue;
        };
        if std::fs::write(&bf, b).is_err() {
            continue;
        }
        let f = lg(
            a,
            &j.repo,
            &[
                "fetch",
                "-q",
                "--no-tags",
                &bfs,
                &format!("+refs/heads/{br}:{pr}"),
            ],
            false,
            600,
        )
        .await;
        if f.c == 0
            && lg(a, &j.repo, &["rev-parse", &pr], false, 60)
                .await
                .o
                .trim()
                == sha
        {
            got = true;
            break;
        }
    }
    let _ = std::fs::remove_file(&bf);
    if !got {
        return e("could not take the commits from the build machine");
    }
    let lr = |o: String| o.split_whitespace().next().unwrap_or("").to_string();
    let rem = lr(lok(
        a,
        &j.repo,
        &["ls-remote", &url, &format!("refs/heads/{br}")],
        true,
        120,
    )
    .await
    .unwrap_or_default());
    if rem != sha {
        let fo = if j.kind == "rebase" || j.kind == "review" {
            format!("--force-with-lease=refs/heads/{br}:{}", j.s("old"))
        } else {
            "--force".into()
        };
        lok(
            a,
            &j.repo,
            &["push", "-q", &fo, &url, &format!("{pr}:refs/heads/{br}")],
            true,
            600,
        )
        .await?;
    }
    let mut r2 = String::new();
    for i in 0..4 {
        if r2 == sha {
            break;
        }
        if i > 0 {
            sl(15_000).await;
        }
        r2 = lr(lok(
            a,
            &j.repo,
            &["ls-remote", &url, &format!("refs/heads/{br}")],
            true,
            120,
        )
        .await
        .unwrap_or_default());
    }
    if r2 != sha {
        return e("push did not arrive");
    }
    // Apenas o SHA confirmado no GitHub pode ser usado por um futuro merge.
    j.d["published_sha"] = json!(sha);
    a.db.jd(j);
    let pn = j.pr.unwrap_or(0);
    if j.kind == "rebase" || j.kind == "review" {
        let u = format!("https://github.com/{}/pull/{pn}", j.repo);
        a.say(
            "submit",
            &format!("PR #{pn} updated ({})", j.kind),
            Some(j.id),
        );
        a.note(
            if j.kind == "rebase" {
                "Rebased"
            } else {
                "Review applied"
            },
            &format!(
                "{}#{pn}: {}\n{u}",
                j.repo,
                f["title"].as_str().unwrap_or("")
            ),
            2,
        );
        return Ok((u, pn));
    }
    let hd = if hr == j.repo { br.clone() } else { format!("{}:{br}", hr.split('/').next().unwrap_or(&me)) };
    let ex = ghj(&["pr", "list", "-R", &j.repo, "--head", &br, "--state", "open", "--json", "number,url,headRepositoryOwner"], 60).await.map_err(X::E)?;
    let ex = ex.as_array().cloned().unwrap_or_default().into_iter().find(|p| p["headRepositoryOwner"]["login"].as_str() == hr.split('/').next());
    let newly_created = ex.is_none();
    let (u, n) = match ex {
        Some(p) => (
            p["url"].as_str().unwrap_or("").to_string(),
            p["number"].as_i64().unwrap_or(0),
        ),
        None => {
            let o = gh(
                &[
                    "pr",
                    "create",
                    "-R",
                    &j.repo,
                    "--head",
                    &hd,
                    "--base",
                    &j.s("mn"),
                    "--title",
                    &ti,
                    "--body",
                    &pb,
                ],
                120,
            )
            .await
            .map_err(X::E)?;
            let u = o.trim().lines().last().unwrap_or("").to_string();
            let n = u
                .rsplit('/')
                .next()
                .and_then(|x| x.parse().ok())
                .unwrap_or(0);
            (u, n)
        }
    };
    let prior = a.db.one("select d from prs where repo=? and num=?", pa![j.repo.clone(), n]);
    let mut pd = prior.as_ref().map(|p| p["d"].clone()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    let enabled = auto_merge_requested(prior.as_ref().map(|p| &p["d"]), newly_created, j, rp(a, &j.repo)["merge"].as_i64().unwrap_or(1) == 1);
    let same_head = pd["head_sha"].as_str() == Some(sha.trim());
    pd["job"] = json!(j.id);
    pd["test_cmd"] = f["test_cmd"].clone();
    pd["tests"] = f["tests"].clone();
    pd["branch"] = json!(br);
    pd["head"] = json!(hr);
    pd["head_sha"] = json!(sha.trim());
    pd["base_branch"] = json!(j.s("mn"));
    pd["auto_merge"] = json!(enabled);
    if enabled {
        if !same_head || pd["merge_status"].is_null() {
            pd["merge_status"] = json!("waiting");
            pd["merge_reason"] = json!("Waiting for GitHub status checks.");
            pd.as_object_mut().unwrap().remove("merge_attempt_head");
            pd.as_object_mut().unwrap().remove("merge_attempt_at");
            pd.as_object_mut().unwrap().remove("merge_enabled_head");
        }
    } else {
        pd["merge_status"] = json!("off");
        pd.as_object_mut().unwrap().remove("merge_reason");
    }
    a.db.run("insert into prs(repo,num,st,title,url,upd,d,t0) values(?,?,?,?,?,?,?,?) on conflict do update set st=excluded.st, d=excluded.d", pa![j.repo.clone(), n, "open".to_string(), ti.clone(), u.clone(), chrono::Utc::now().to_rfc3339(), pd.to_string(), now()]);
    a.say("submit", &format!("PR opened: {u}"), Some(j.id));
    a.note("PR opened", &format!("{} → {ti}\n{u}", j.tag()), 3);
    Ok((u, n))
}

async fn clean(a: &Ax, j: &Job) {
    hx(a, &j.h(), &format!("cd \"{}\" 2>/dev/null && {{ git rebase --abort; git merge --abort; }} 2>/dev/null; cd \"{}\" 2>/dev/null && git reset -q --hard && git clean -fdq && git checkout -q --detach origin/HEAD 2>/dev/null; true", dir(a, j), dir(a, j)), 600).await;
}

async fn stage<T, F, Fu>(a: &Ax, w: &str, mut f: F) -> R<T>
where
    F: FnMut() -> Fu,
    Fu: std::future::Future<Output = R<T>>,
{
    let mut k = 0;
    loop {
        match f().await {
            Ok(v) => return Ok(v),
            Err(X::Lim(m)) | Err(X::Net(m)) if k < 8 && !crate::routing::enabled(a) => {
                k += 1;
                a.say(w, &format!("waiting: {}", crate::sh::head(&m, 120)), None);
                let _ = need(a, w, &["gh", "an"]).await;
                sl(if m.to_lowercase().contains("limit") {
                    600_000
                } else {
                    30_000
                })
                .await;
            }
            Err(x) => return Err(x),
        }
    }
}

async fn hunt(a: &Ax, j: &mut Job) -> R<i64> {
    a.db.jset(j.id, &[("st", json!("hunt"))]);
    a.st("hunter", "work", Some(j.id), "");
    a.say(
        "hunter",
        &format!("[{}] hunting bugs in {}", j.h(), j.repo),
        Some(j.id),
    );
    let mut ti: Vec<String> = a.db.all("select id, json_extract(d,'$.title') t, coalesce(json_extract(d,'$.hunt.where'),json_extract(d,'$.ext.where'),'') w from jobs where repo=? and num=0 and kind='fix' and st not in ('drop') order by id desc limit 200", pa![j.repo.clone()]).iter().map(|x| format!("office find #{}: {}\n  where: {}", x["id"], x["t"].as_str().unwrap_or(""), x["w"].as_str().unwrap_or(""))).collect();
    for x in ghj(
        &[
            "pr",
            "list",
            "-R",
            &j.repo,
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            "number,title,body",
        ],
        240,
    )
    .await
    .unwrap_or_default()
    .as_array()
    .cloned()
    .unwrap_or_default()
    {
        ti.push(format!(
            "pr #{}: {}\n  {}",
            x["number"],
            x["title"].as_str().unwrap_or(""),
            crate::sh::head(
                &x["body"]
                    .as_str()
                    .unwrap_or("")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                250
            )
        ));
    }
    for x in ghj(
        &[
            "issue",
            "list",
            "-R",
            &j.repo,
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            "number,title",
        ],
        240,
    )
    .await
    .unwrap_or_default()
    .as_array()
    .cloned()
    .unwrap_or_default()
    {
        ti.push(format!(
            "issue #{}: {}",
            x["number"],
            x["title"].as_str().unwrap_or("")
        ));
    }
    let md = a.md("hunter");
    let mut o = Ask::new(
        "hunter",
        md.clone(),
        P("hunter"),
        format!(
            "Repository: {}.\n\nAlready reported or in progress (do not report these again):\n{}{}",
            j.repo,
            wrap("known", &crate::sh::head(&ti.join("\n"), 200_000)),
            rules(a, j)
        ),
    );
    o.job = Some(j.id);
    o.rn = rn(a, j);
    o.turns = 100;
    o.t = 90 * 60;
    let (_, js) = ask(a, o).await?;
    x(a, j, "git checkout -q -- . && git clean -fdq", 120).await;
    let fs: Vec<Value> = js["finds"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|x| x["title"].is_string() && x["desc"].is_string() && x["where"].is_string())
        .take(1)
        .collect();
    a.st(
        "hunter",
        "idle",
        None,
        if fs.is_empty() { "sad" } else { "ok" },
    );
    if fs.is_empty() {
        a.say("hunter", &format!("{}: nothing proven", j.repo), Some(j.id));
        return Ok(0);
    }
    let g = crate::wk::guard(
        a,
        fs.iter()
            .enumerate()
            .map(|(k, x)| {
                (
                    format!("h{k}"),
                    format!(
                        "{}\n{}\n{}\n{}",
                        x["title"].as_str().unwrap_or(""),
                        x["where"].as_str().unwrap_or(""),
                        x["desc"].as_str().unwrap_or(""),
                        x["proof"].as_str().unwrap_or("")
                    ),
                )
            })
            .collect(),
        "hunter",
        Some(j.id),
    )
    .await?;
    let mut n = 0;
    for (k, x) in fs.iter().enumerate() {
        if g[&format!("h{k}")]["inj"].as_bool().unwrap_or(false) {
            a.say(
                "guard",
                &format!(
                    "hunter find blocked: {}",
                    g[&format!("h{k}")]["why"].as_str().unwrap_or("")
                ),
                Some(j.id),
            );
            continue;
        }
        let t = x["title"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        a.db.jnew(&j.repo, 0, "fix", &t, &format!("https://github.com/{}", j.repo), "vet", "found by Hunter", json!({"ms": [md], "hunt": {"title": x["title"], "where": x["where"], "proof": x["proof"]}, "title": x["title"], "body": format!("{}\n\nWhere: {}\nProof: {}", x["desc"].as_str().unwrap_or(""), x["where"].as_str().unwrap_or(""), x["proof"].as_str().unwrap_or("-"))}), None, 0.0);
        a.flow("hunter", "guard", &j.repo);
        n += 1;
    }
    a.say(
        "hunter",
        &format!("{}: {n} bug(s) found, sent to Vet", j.repo),
        Some(j.id),
    );
    Ok(n)
}

async fn desc(a: &Ax, j: &mut Job) -> R<Value> {
    let df = ok(a, j, &format!("git diff {}..HEAD | head -c 50000", j.s("base")), 120).await?;
    let lg = ok(a, j, &format!("git log --format='%s%n%b' {}..HEAD", j.s("base")), 60).await?;
    let mut o = Ask::new("fixer", a.md("reviewer"), P("desc"), format!("Repository {}. {}\n\nProof lines from the verification run:\n{}\n\nCommits:\n{lg}\n\nDiff:\n{df}\n\nAnswer with {{\"title\": \"PR title, imperative, max 72 chars\", \"cause\": \"root cause or concrete inefficiency, 1-3 sentences\", \"fix\": \"what changed, 1-3 sentences\", \"untouched\": \"behavior or areas preserved, or empty\", \"tests\": [\"exact names of the checks used for this job type\"], \"test_cmd\": \"exact shell command from the repo root that runs only those checks\"}}", j.repo, wrap("issue", &format!("{}\n\n{}", j.title, crate::sh::head(&j.s("body"), 3000))), j.d["proof"]));
    o.job = Some(j.id);
    o.ef = Some("medium");
    let (_, js) = ask(a, o).await?;
    if !js["title"].is_string()
        || js["tests"].as_array().map(|x| x.is_empty()).unwrap_or(true)
        || !js["test_cmd"].is_string()
    {
        return e("could not reconstruct the fix description");
    }
    Ok(js)
}

async fn resume(a: &Ax, j: &mut Job) -> R<Value> {
    a.say(
        "ci",
        &format!("[{}] resuming {} after a restart", j.h(), j.tag()),
        Some(j.id),
    );
    ok(a, j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
    let mut f = j.d["f0"].clone();
    if f.is_null() {
        f = desc(a, j).await?;
        j.d["f0"] = f.clone();
        a.db.jd(j);
    }
    let k = j.d["k"].as_u64().unwrap_or(0);
    let mut fb = match crate::quality::check(a, j).await {
        Ok(value) => value,
        Err(e) => {
            fix_attempt_finish_active(a, j, "failed", "verification");
            return Err(e);
        }
    };
    if !fb.is_empty() {
        fix_attempt_finish_active(a, j, "rejected", "verification");
    }
    if fb.is_empty() {
        fb = match ci(a, j).await {
            Ok(value) => value,
            Err(e) => {
                fix_attempt_finish_active(a, j, "failed", "ci");
                return Err(e);
            }
        };
    }
    if fb.starts_with("CI aborted after") {
        fix_attempt_finish_active(a, j, "blocked", "ci");
        return drop(format!("{fb} on {}", j.h()));
    }
    if !fb.is_empty() {
        fix_attempt_finish_active(a, j, "rejected", "ci");
    }
    if fb.is_empty() {
        fb = match review(a, j, &mut f).await {
            Ok(value) => value,
            Err(e) => {
                fix_attempt_finish_active(a, j, "failed", "review");
                return Err(e);
            }
        };
        if !fb.is_empty() {
            fix_attempt_finish_active(a, j, "rejected", "review");
        }
    }
    if !fb.is_empty() {
        return fix(a, j, fb, k + 1).await;
    }
    fix_attempt_finish_active(a, j, "accepted", "");
    j.d["f"] = f.clone();
    a.db.jd(j);
    Ok(f)
}

fn idle_all(a: &Ax, id: i64) {
    for w in ["repro", "fixer", "ci", "reviewer", "submit", "hunter"] {
        let b =
            a.lv.lock()
                .unwrap()
                .w
                .get(w)
                .map(|x| x.job == Some(id))
                .unwrap_or(false);
        if b {
            a.st(w, "idle", None, "");
        }
    }
}

/// Revalidates an existing PR without creating commits or publishing changes.
async fn mergecheck(a: &Ax, j: &mut Job) -> R<()> {
    let head = j.s("expected_head");
    let base = j.s("expected_base");
    let branch = j.s("branch");
    let valid_sha = |s: &str| s.len() == 40 && s.bytes().all(|c| c.is_ascii_hexdigit());
    if !valid_sha(&head) || !valid_sha(&base) || branch.is_empty() || j.pr.is_none() || j.s("author").is_empty() {
        return drop("provas insuficientes para revalidar PR");
    }
    let number = j.pr.unwrap_or(0);
    let before = ghj(&["api", &format!("repos/{}/pulls/{number}", j.repo)], 45).await.map_err(X::E)?;
    if before["head"]["sha"] != head || before["base"]["sha"] != base || before["head"]["ref"] != branch
        || before["head"]["repo"]["full_name"] != j.repo || before["state"] != "open" || before["draft"] != false {
        return drop("PR mudou antes de executar testes locais");
    }
    prep(a, j).await?;
    let actual_head = ok(a, j, "git rev-parse HEAD", 60).await?;
    let main = crate::gh::meta(a, &j.repo).await.map_err(X::E)?["main"].as_str().unwrap_or("main").to_string();
    let actual_base = ok(a, j, &format!("git rev-parse origin/{}", q(&main)), 60).await?;
    if actual_head != head || actual_base != base { return drop("checkout da PR ou da main difere dos SHAs esperados"); }

    // Reviewer compara a alteração com a base e consulta o conteúdo da PR.
    j.d["base"] = json!(base);
    j.d["body"] = before["body"].clone();
    a.db.jd(j);
    let ci_result = ci(a, j).await?;
    if !ci_result.is_empty() { return drop(format!("CI reprovada na revalidação: {}", crate::sh::head(&ci_result, 700))); }
    let mut report = json!({"title":j.title,"tests":[],"test_cmd":rp(a,&j.repo)["ci"],"cause":"Revisão de integração existente","fix":"Código existente sob revisão"});
    let feedback = review(a, j, &mut report).await?;
    if !feedback.is_empty() { return drop(format!("Reviewer reprovou: {}", crate::sh::head(&feedback, 700))); }

    let actual_head = ok(a, j, "git rev-parse HEAD", 60).await?;
    let tree = ok(a, j, "git rev-parse HEAD^{tree}", 60).await?;
    let fresh = ghj(&["api", &format!("repos/{}/pulls/{number}", j.repo)], 45).await.map_err(X::E)?;
    if actual_head != head || j.d["ci_tree"] != tree || fresh["head"]["sha"] != head || fresh["base"]["sha"] != base
        || fresh["state"] != "open" || fresh["draft"] != false {
        return drop("PR mudou entre testes, revisão e revalidação final");
    }
    j.d["published_sha"] = json!(head);
    j.d["publish_tree"] = json!(tree);
    a.db.jd(j);
    a.db.jset(j.id, &[("st", json!("done")), ("why", json!("revalidação local concluída; aguarda gate de merge"))]);
    a.say("reviewer", &format!("{}#{number}: CI e revisão independente aprovadas no HEAD atual", j.repo), Some(j.id));
    clean(a, j).await;
    a.steward_wake.notify_one();
    Ok(())
}

async fn go(a: &Ax, id: i64, h: &str) {
    let Some(mut j) = a.db.jg(id) else { return };
    let rs = j.d["rs"].as_bool().unwrap_or(false);
    let rpp = j.d["rp"].as_bool().unwrap_or(false);
    if (j.st != "submit" && !rs && !rpp) || j.h().is_empty() {
        j.d["h"] = json!(h);
        a.db.jd(&j);
    }
    a.say("repro", &format!("[{h}] takes {}", j.tag()), Some(id));
    let r: R<()> = async {
        if j.kind == "mergecheck" {
            mergecheck(a, &mut j).await?;
            return Ok(());
        }
        if j.st == "submit" {
            ok(a, &j, &format!("git checkout -q {}", q(&j.s("br"))), 120).await?;
            stage(a, "submit", || {
                let mut jj = j.clone();
                async move {
                    gate(a, &mut jj).await?;
                    Ok(jj)
                }
            })
            .await
            .map(|x| j = x)?;
            if j.kind == "fix" && j.s("base") != j.s("pbase") {
                j.d["df"] = json!(ok(
                    a,
                    &j,
                    &format!("git diff {}..HEAD | head -c 120000", j.s("base")),
                    120
                )
                .await
                .unwrap_or_default());
                a.db.jd(&j);
            }
        } else if rs {
            j.d.as_object_mut().unwrap().remove("rs");
            a.db.jd(&j);
            let mut jj = j.clone();
            resume(a, &mut jj).await?;
            j = jj;
            kill(a, &j)?;
            let mut jj = j.clone();
            gate(a, &mut jj).await?;
            j = jj;
            kill(a, &j)?;
        } else {
            let mut jj = j.clone();
            stage(a, "repro", || {
                let mut k = jj.clone();
                async move {
                    prep(a, &mut k).await?;
                    Ok(k)
                }
            })
            .await
            .map(|x| jj = x)?;
            j = jj;
            crate::quality::inspect(a, &mut j).await?;
            kill(a, &j)?;
            if j.kind == "hunt" {
                let mut jj = j.clone();
                let n = stage(a, "hunter", || {
                    let mut k = jj.clone();
                    async move { hunt(a, &mut k).await }
                })
                .await?;
                jj = a.db.jg(id).unwrap_or(jj);
                a.db.jset(
                    id,
                    &[
                        ("st", json!("done")),
                        ("why", json!(format!("{n} bug(s) found"))),
                    ],
                );
                clean(a, &jj).await;
                return Err(X::Back(String::new()));
            }
            for k in [
                format!("cired:{}", j.repo),
                format!("cired:{}{}", j.repo, hk(a, &j.h())),
            ] {
                if let Some(cr) = a.db.kg(&k).as_object() {
                    if cr.get("sha").and_then(|x| x.as_str()) == Some(j.s("base").as_str()) {
                        return Err(X::Back(format!(
                            "{}: CI is red on the default branch, waiting until it moves",
                            j.repo
                        )));
                    }
                    a.db.kd(&k);
                }
            }
            if j.kind == "fix" {
                stage(a, "repro", || race(a, &j, "before repro")).await?;
                let mut jj = j.clone();
                stage(a, "repro", || {
                    let mut k = jj.clone();
                    async move {
                        repro(a, &mut k).await?;
                        Ok(k)
                    }
                })
                .await
                .map(|x| jj = x)?;
                j = jj;
            }
            kill(a, &j)?;
            stage(a, "fixer", || race(a, &j, "before fix")).await?;
            let mut jj = j.clone();
            if j.kind == "rebase" {
                stage(a, "fixer", || {
                    let mut k = jj.clone();
                    async move {
                        reb(a, &mut k).await?;
                        Ok(k)
                    }
                })
                .await
                .map(|x| jj = x)?;
            } else {
                stage(a, "fixer", || {
                    let mut k = jj.clone();
                    async move {
                        fix(a, &mut k, String::new(), 0).await?;
                        Ok(k)
                    }
                })
                .await
                .map(|x| jj = x)?;
            }
            j = jj;
            kill(a, &j)?;
            let mut jj = j.clone();
            stage(a, "submit", || {
                let mut k = jj.clone();
                async move {
                    gate(a, &mut k).await?;
                    Ok(k)
                }
            })
            .await
            .map(|x| jj = x)?;
            j = jj;
            kill(a, &j)?;
            mkbody(a, &mut j).await?;
            j.d["pbase"] = json!(j.s("base"));
            a.db.jd(&j);
            let auto = !crate::routing::enabled(a)
                && (rp(a, &j.repo)["mode"].as_str() == Some("auto") || j.kind != "fix");
            if !auto && !j.d["ap"].as_bool().unwrap_or(false) {
                a.db.jset(
                    id,
                    &[
                        ("st", json!("approve")),
                        ("why", json!("waiting for your approval")),
                    ],
                );
                a.st("submit", "idle", None, "ok");
                a.say(
                    "submit",
                    &format!("{} is ready and waits for your approval", j.tag()),
                    Some(id),
                );
                a.note(
                    "PR waits for approval",
                    &format!("{}: {}", j.tag(), j.d["pt"].as_str().unwrap_or("")),
                    3,
                );
                clean(a, &j).await;
                return Err(X::Back(String::new()));
            }
        }
        let mut jj = j.clone();
        let (u, n) = stage(a, "submit", || {
            let mut k = jj.clone();
            async move {
                let r = submit(a, &mut k).await?;
                Ok((r, k))
            }
        })
        .await
        .map(|(r, k)| {
            jj = k;
            r
        })?;
        j = jj;
        a.db.jset(
            id,
            &[
                ("st", json!("done")),
                ("pr", if n > 0 { json!(n) } else { Value::Null }),
                (
                    "why",
                    json!(if u.is_empty() { "DRY".to_string() } else { u }),
                ),
            ],
        );
        if n > 0 {
            // Wake after persisting the published PR, before repository cleanup.
            // Cleanup can be slow; the Steward still applies its normal review gates.
            a.steward_wake.notify_one();
            a.say(
                "submit",
                &format!(
                    "{}: PR #{n} publicada; aguarda revisão e integração",
                    j.repo
                ),
                Some(id),
            );
        }
        a.st("submit", "idle", None, "ok");
        clean(a, &j).await;
        Ok(())
    }
    .await;
    match r {
        Ok(()) => {}
        Err(X::Back(m)) => {
            if !m.is_empty() {
                a.db.jset(id, &[("st", json!("ready"))]);
                a.say(
                    "ci",
                    &format!("{m}, {} goes back to the queue", j.tag()),
                    Some(id),
                );
                clean(a, &j).await;
            }
            idle_all(a, id);
        }
        Err(X::Host(hh, m)) => {
            crate::net::down(a, &hh);
            let mut jj = a.db.jg(id).unwrap_or(j.clone());
            let n = jj.d["ir"].as_i64().unwrap_or(0) + 1;
            idle_all(a, id);
            if n > 5 {
                let detail = crate::hunt_health::safe_tail(&m);
                crate::hunt_health::host_diagnostic(&mut jj, &hh, &detail);
                a.db.jset(
                    id,
                    &[
                        ("st", json!("drop")),
                        ("d", jj.d.clone()),
                        (
                            "why",
                            json!(format!(
                                "machine kept failing: {}",
                                crate::sh::head(&crate::hunt_health::safe_tail(&m), 260)
                            )),
                        ),
                    ],
                );
                a.say(
                    "submit",
                    &format!(
                        "{} dropped after {} machine failures: {}",
                        jj.tag(),
                        n - 1,
                        crate::sh::head(&crate::hunt_health::safe_tail(&m), 180)
                    ),
                    Some(id),
                );
            } else {
                jj.d["ir"] = json!(n);
                let pin = jj.st == "submit"
                    || jj.d["rs"].as_bool().unwrap_or(false)
                    || jj.d["rp"].as_bool().unwrap_or(false);
                if !pin {
                    jj.d.as_object_mut().map(|o| o.remove("h"));
                }
                a.db.jset(
                    id,
                    &[
                        (
                            "st",
                            json!(if jj.st == "submit" { "submit" } else { "ready" }),
                        ),
                        ("d", jj.d.clone()),
                    ],
                );
                a.say(
                    "ci",
                    &format!(
                        "[{hh}] machine unavailable, {} goes back to the queue: {}",
                        jj.tag(),
                        crate::sh::head(&m, 160)
                    ),
                    Some(id),
                );
            }
        }
        Err(x) => {
            let m = match &x {
                X::Drop(m) => m.clone(),
                o => format!("error: {}", crate::sh::head(&o.to_string(), 300)),
            };
            let m = crate::hunt_health::safe_tail(&m);
            let mut jj = a.db.jg(id).unwrap_or(j.clone());
            let stage = jj.st.clone();
            let host = jj.h();
            jj.d["failure"] = failure_details(&stage, &host, &jj.d, &m, now());
            a.db.jset(
                id,
                &[
                    ("st", json!("drop")),
                    ("why", json!(crate::sh::last(&m, 300))),
                    ("d", jj.d.clone()),
                ],
            );
            idle_all(a, id);
            a.say("submit", &format!("{} dropped: {m}", jj.tag()), Some(id));
            if Regex::new(r"faster|conflict|error|rounds|gave up|CI")
                .unwrap()
                .is_match(&m)
                && jj.kind != "hunt"
            {
                a.note(
                    "Dropped",
                    &format!("{}: {m}", jj.tag()),
                    if m.starts_with("error") { 4 } else { 2 },
                );
            }
            clean(a, &jj).await;
        }
    }
}

pub fn recover(a: &Ax) {
    for j in a.db.all("select id, st, kind, d from jobs where st in ('repro','fix','ci','review','gate','hunt','submit')", pa![]) {
        let id = j["id"].as_i64().unwrap_or(0);
        let mut d = j["d"].clone();
        let st = j["st"].as_str().unwrap_or("");
        let kd = j["kind"].as_str().unwrap_or("");
        if st == "submit" && d["ap"].as_bool().unwrap_or(false) {
            continue;
        }
        if kd == "fix" && ["ci", "review", "gate", "submit"].contains(&st) && d["h"].is_string() && d["br"].is_string() {
            d["rs"] = json!(true);
        } else if (kd == "fix" || kd == "hunt") && d["h"].is_string() {
            d["rp"] = json!(true);
        }
        a.db.jset(id, &[("st", json!("ready")), ("d", d), ("prio", json!(-2))]);
    }
}

fn hk(a: &Ax, h: &str) -> String {
    if crate::sh::ho(a, h).map(|x| x.dk).unwrap_or(true) {
        String::new()
    } else {
        format!("@{h}")
    }
}

async fn red(a: &Ax, r: &str, h: &str) -> bool {
    let mut cr = a.db.kg(&format!("cired:{r}"));
    if cr.is_null() {
        let x = hk(a, h);
        if !x.is_empty() {
            cr = a.db.kg(&format!("cired:{r}{x}"));
        }
    }
    if cr.is_null() {
        return false;
    }
    let k = format!("head:{r}");
    let h = match crate::gh::cg(a, &k, 600_000) {
        Some(v) => v,
        None => {
            let v = ghj(
                &[
                    "api",
                    &format!("repos/{r}/commits/HEAD"),
                    "--jq",
                    "{s: .sha}",
                ],
                60,
            )
            .await
            .unwrap_or(Value::Null);
            crate::gh::cs(a, &k, &v);
            v
        }
    };
    !(h["s"].is_string() && cr["sha"].is_string() && h["s"] != cr["sha"])
}

// Atomically reserve work against updates, cache maintenance and other lanes.
pub(crate) fn admit(a: &Ax, repo: &str, host: &str, id: i64) -> bool {
    let _transition = a.update_transition.lock().unwrap();
    if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    let is_hunt = a.db.jg(id).is_some_and(|job| job.kind == "hunt");
    if is_hunt && crate::hunt_health::blocked(a, repo) {
        return false;
    }
    let hunter_limit = if is_hunt { hn(a) } else { 0 };
    if docker_cache_target(a, host)
        .is_some_and(|target| a.cache_maintenance.lock().unwrap().contains(&target))
    {
        return false;
    }
    let mut active = a.act.lock().unwrap();
    let mut locked = a.lk.lock().unwrap();
    let claim = format!("{repo} {host}");
    if active.contains(&claim) || locked.contains(&id) {
        return false;
    }
    if is_hunt
        && locked
            .iter()
            .filter(|other_id| a.db.jg(**other_id).is_some_and(|job| job.kind == "hunt"))
            .count() as i64
            >= hunter_limit
    {
        return false;
    }
    // Hunter and Fixer share a repository checkout, even across hosts.
    let active_repo = active
        .iter()
        .any(|entry| entry.split_once(' ').is_some_and(|(r, _)| r == repo));
    if active_repo
        && (is_hunt
            || locked.iter().any(|other_id| {
                a.db.jg(*other_id)
                    .is_some_and(|job| job.repo == repo && job.kind == "hunt")
            }))
    {
        return false;
    }
    active.insert(claim);
    locked.insert(id);
    true
}

fn pick_rank(j: &Value) -> (i32, i32, i32, f64, i64, i64) {
    let d = &j["d"];
    let kind = j["kind"].as_str().unwrap_or("fix");
    let resumed = d["rs"].as_bool().unwrap_or(false) || d["rp"].as_bool().unwrap_or(false);
    let phase = if j["st"] == "submit" {
        0
    } else if resumed {
        1
    } else if matches!(kind, "review" | "mergecheck") {
        2
    } else if kind == "fix" {
        3
    } else if kind == "rebase" {
        4
    } else if kind == "hunt" {
        5
    } else {
        6
    };
    let follow_up_order = match kind {
        "review" => 0,
        "rebase" => 1,
        "mergecheck" => 2,
        _ => 0,
    };
    let issue_priority = d["issue_priority"].as_i64().unwrap_or(2).clamp(0, 3) as i32;
    (
        phase,
        follow_up_order,
        issue_priority,
        j["prio"].as_f64().unwrap_or(0.0),
        j["t0"].as_i64().unwrap_or(i64::MAX),
        j["id"].as_i64().unwrap_or(0),
    )
}

/// Selects eligible work, reserving the final admission under update_transition.
async fn pick(a: &Ax, h: &str) -> Option<i64> {
    if a.drain.load(std::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let mut c = a.db.all("select * from jobs where st in ('ready','submit') order by id", pa![]);
    c.sort_by(|x, y| pick_rank(x).partial_cmp(&pick_rank(y)).unwrap_or(std::cmp::Ordering::Equal));
    for j in c {
        let id = j["id"].as_i64().unwrap_or(0);
        let r = j["repo"].as_str().unwrap_or("").to_string();
        let d = &j["d"];
        let pin = j["st"] == "submit"
            || d["rs"].as_bool().unwrap_or(false)
            || d["rp"].as_bool().unwrap_or(false);
        if a.act.lock().unwrap().contains(&format!("{r} {h}")) || a.lk.lock().unwrap().contains(&id)
        {
            continue;
        }
        if j["kind"] == "hunt" && (a.hold("hunter") || crate::hunt_health::blocked(a, &r)) {
            continue;
        }
        if !pin && (j["kind"] == "rebase" || j["kind"] == "review") {
            let tl =
                a.db.n("select coalesce(sum(ln),0) from hs where on_=1", pa![]);
            let rr = a.db.n("select count(*) from jobs where kind in ('rebase','review') and st not in ('ready','done','drop','vet','park','approve')", pa![]);
            if rr >= (tl / 2).max(1) {
                continue;
            }
        }
        if pin && d["h"].as_str().map(|x| x != h).unwrap_or(false) {
            continue;
        }
        if !pin && crate::sh::ho(a, h).map(|x| x.skips(&r)).unwrap_or(false) {
            continue;
        }
        let rr = rp(a, &r);
        if rr["on_"].as_i64().unwrap_or(0) != 1 && !pin {
            continue;
        }
        if rr["ci"].as_str().unwrap_or("").trim().is_empty() && !pin {
            continue;
        }
        if !pin && red(a, &r, h).await {
            continue;
        }
        if !pin && j["kind"] == "fix" {
            let mo = rr["maxo"].as_i64().unwrap_or(3);
            let md = rr["maxd"].as_i64().unwrap_or(5);
            let open = if mo > 0 {
                crate::gh::opened(a, &r).await + a.db.n("select count(*) from jobs where repo=? and kind='fix' and st in ('repro','fix','ci','review','gate','submit')", pa![r.clone()])
            } else {
                0
            };
            let daily = if md > 0 { crate::gh::today(a, &r).await } else { 0 };
            if repo_limit_reached(open, daily, mo, md) {
                continue;
            }
        }
        if admit(a, &r, h, id) {
            return Some(id);
        }
        if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
    }
    None
}

fn repo_limit_reached(open: i64, daily: i64, max_open: i64, max_daily: i64) -> bool {
    (max_open > 0 && open >= max_open) || (max_daily > 0 && daily >= max_daily)
}

pub fn bl(a: &Ax, r: &str) -> i64 {
    if !a.db.kg(&format!("cired:{r}")).is_null() {
        return 0;
    }
    hosts(a)
        .iter()
        .filter(|h| {
            h.on && a.net(&h.id)
                && !h.skips(r)
                && (h.dk || a.db.kg(&format!("cired:{r}@{}", h.id)).is_null())
        })
        .map(|h| h.ln)
        .sum()
}

pub fn qn(a: &Ax, r: &str, sts: &str) -> i64 {
    a.db.n(
        &format!("select count(*) from jobs where repo=? and kind='fix' and st in ({sts})"),
        pa![r.to_string()],
    )
}

fn hn(a: &Ax) -> i64 {
    if crate::routing::enabled(a) {
        return 0;
    }
    let cap = a.set()["hunters"].as_i64().unwrap_or(2).clamp(0, 12);
    let l: i64 = hosts(a)
        .iter()
        .filter(|h| h.on && a.net(&h.id))
        .map(|h| h.ln)
        .sum();
    let hf: i64 = a.db.all("select repo, count(*) n from jobs where kind='fix' and num=0 and st in ('vet','ready') and json_extract(d,'$.ext') is null group by repo", pa![]).iter().map(|x| x["n"].as_i64().unwrap_or(0).min(2 * bl(a, x["repo"].as_str().unwrap_or("")))).sum();
    if cap == 0 || l == 0 {
        return 0;
    }
    // hf already reserves lanes for eligible vet/ready fixes. Count only
    // distinct work not covered by hf; otherwise a single ready fix
    // reserves two lanes and can unnecessarily suppress Hunters.
    let uncovered_fixes = a.db.n(
        "select count(*) from jobs where kind='fix' and (st in ('repro','fix','ci','review','gate','submit') or (st='ready' and (num<>0 or json_extract(d,'$.ext') is not null)))",
        pa![],
    );
    let active_rechecks = a.db.n(
        "select count(*) from jobs j join rp r on r.full=j.repo where j.kind='mergecheck' and j.st not in ('done','drop','park') and r.on_=1",
        pa![],
    );
    let reserved = hf + i64::from(uncovered_fixes > 0) + i64::from(active_rechecks > 0);
    (l - reserved.max(0)).clamp(0, cap)
}

pub fn hy(a: &Ax, r: &str) -> f64 {
    let t = now() - 14 * 86_400_000;
    let h = a.db.n(
        "select count(*) from jobs where repo=? and kind='hunt' and t0>?",
        pa![r.to_string(), t],
    );
    let m = a.db.n("select count(*) from jobs j join prs p on p.repo=j.repo and p.num=j.pr where j.repo=? and j.kind='fix' and j.num=0 and j.t0>? and p.st='merged'", pa![r.to_string(), t]);
    let s = a.db.n(
        "select count(*) from prs where repo=? and st='open' and t0<?",
        pa![r.to_string(), now() - 7 * 86_400_000],
    );
    (m as f64 + 1.0) / (h as f64 + 2.0) / (1.0 + s as f64 / 10.0)
}

/// Cria no máximo um Hunter elegível por repositório, com gate serializado.
fn mk_hunt(a: &Ax) -> bool {
    // Coordinate creation with admission and breaker updates across lanes.
    let _transition = a.update_transition.lock().unwrap();
    if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    let on = a.db.all(
        "select repo, st from jobs where kind='hunt' and st not in ('done','drop','park')",
        pa![],
    );
    // A queued hunt blocked by the circuit breaker cannot use a lane.
    // Keep it in 'on' for duplicate suppression, but exclude it from
    // active capacity until its repository is no longer blocked.
    let occupying = on
        .iter()
        .filter(|j| {
            j["st"] != "ready" || !crate::hunt_health::blocked(a, j["repo"].as_str().unwrap_or(""))
        })
        .count() as i64;
    if occupying >= hn(a) {
        return false;
    }
    let d0 = crate::db::day0();
    let rs: Vec<String> =
        a.db.all(
            "select full from rp where on_=1 and hunt=1 and ci<>''",
            pa![],
        )
        .iter()
        .filter_map(|x| x["full"].as_str().map(String::from))
        .collect();
    let act: Vec<String> = a
        .act
        .lock()
        .unwrap()
        .iter()
        .map(|k| k.split(' ').next().unwrap_or("").to_string())
        .collect();
    let cnt = |r: &str| {
        a.db.n(
            "select count(*) from jobs where kind='hunt' and repo=? and t0>?",
            pa![r.to_string(), d0],
        )
    };
    let mut c: Vec<String> = rs
        .into_iter()
        .filter(|r| {
            !crate::hunt_health::blocked(a, r)
                && !act.contains(r)
                && !on.iter().any(|x| x["repo"].as_str() == Some(r.as_str()))
                && {
                    let b = bl(a, r);
                    b > 0 && qn(a, r, "'vet','ready'") < 4 * b
                }
        })
        .collect();
    let sc = |r: &str| (cnt(r) as f64 + 1.0) / hy(a, r);
    c.sort_by(|x, y| {
        sc(x)
            .partial_cmp(&sc(y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let Some(r) = c.first() else { return false };
    a.db.jnew(
        r,
        0,
        "hunt",
        &format!("Hunt for bugs in {r}"),
        &format!("https://github.com/{r}"),
        "ready",
        "hunter slot",
        json!({}),
        None,
        99.0,
    );
    a.say("hunter", &format!("starting a hunt in {r}"), None);
    true
}

async fn lane(a: Ax, h: String, k: i64) {
    let key = format!("{h}#{k}");
    while let Some(x) = ho(&a, &h) {
        if !x.on || k > x.ln {
            break;
        }
        for w in ["repro", "fixer", "submit"] {
            a.held(w).await;
        }
        if !a.net(&h) {
            sl(60_000).await;
            continue;
        }
        // Capacity reservation: when a host has two or more lanes, the top lane
        // never starts new hunts, so fixes and PR rechecks keep a lane and are
        // never starved by continuous hunting.
        let reserve_for_fixes = x.ln >= 2 && k == x.ln;
        while !reserve_for_fixes
            && !a.hold("hunter")
            && !a.drain.load(std::sync::atomic::Ordering::Relaxed)
            && mk_hunt(&a)
        {}
        let Some(id) = pick(&a, &h).await else {
            sl(20_000).await;
            continue;
        };
        let r = a.db.jg(id).map(|j| j.repo).unwrap_or_default();
        a.lv.lock().unwrap().ln.insert(id, key.clone());
        go(&a, id, &h).await;
        a.lv.lock().unwrap().ln.remove(&id);
        a.act.lock().unwrap().remove(&format!("{r} {h}"));
        a.lk.lock().unwrap().remove(&id);
    }
    a.lanes.lock().unwrap().remove(&key);
}

static ENS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);
static ING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

/// Diagnostic snapshot of the build-container readiness per host:
/// returns (ensured, preparing) host ids so the UI can distinguish a host
/// that is ready from one still running `ensure` (the "preparing the build
/// container" step) instead of leaving it looking silently idle.
pub fn ensure_state() -> (Vec<String>, Vec<String>) {
    let ens = ENS.lock().unwrap().iter().cloned().collect();
    let ing = ING.lock().unwrap().iter().cloned().collect();
    (ens, ing)
}

pub async fn lanes(a: Ax) {
    loop {
        for x in hosts(&a) {
            if !x.on {
                continue;
            }
            if x.dk && !ENS.lock().unwrap().contains(&x.id) {
                if ING.lock().unwrap().insert(x.id.clone()) {
                    let (a2, id) = (a.clone(), x.id.clone());
                    tokio::spawn(async move {
                        a2.say("ci", &format!("[{id}] preparing the build container"), None);
                        match crate::sh::ensure(&a2, &id).await {
                            Ok(()) => {
                                a2.db.ks(&format!("up:{id}"), &json!(true));
                                ENS.lock().unwrap().insert(id.clone());
                            }
                            Err(m) => a2.say("ci", &format!("[{id}] machine not ready: {m}"), None),
                        }
                        ING.lock().unwrap().remove(&id);
                    });
                }
                continue;
            }
            for k in 1..=x.ln {
                let key = format!("{}#{k}", x.id);
                let new = a.lanes.lock().unwrap().insert(key);
                if new {
                    tokio::spawn(lane(a.clone(), x.id.clone(), k));
                    sl(3000).await;
                }
            }
        }
        sl(30_000).await;
    }
}

async fn disk(a: &Ax, h: &crate::sh::Ho) -> Out {
    let ac: Vec<String> = a
        .act
        .lock()
        .unwrap()
        .iter()
        .filter(|k| k.ends_with(&format!(" {}", h.id)))
        .map(|k| slug(k.split(' ').next().unwrap_or("")))
        .collect();
    let pn: Vec<String> =
        a.db.all(
            "select repo, d from jobs where st in ('ready','submit','approve')",
            pa![],
        )
        .iter()
        .filter(|y| y["d"]["h"].as_str() == Some(h.id.as_str()))
        .map(|y| slug(y["repo"].as_str().unwrap_or("")))
        .collect();
    let ex = |l: &[String]| {
        std::iter::once("-e __none__/".to_string())
            .chain(l.iter().map(|r| format!("-e {}", q(&format!("{r}/")))))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut ap = ac.clone();
    ap.extend(pn);
    let s = format!(
        "cd \"{b}\" 2>/dev/null || exit 0\nfr() {{ df -Pk . | awk 'NR==2{{print int($4/1048576)}}'; }}\nus() {{ du -sk . 2>/dev/null | awk '{{print int($1/1048576)}}'; }}\ngd() {{ [ \"$(fr)\" -ge 20 ] && [ \"$(us)\" -le {gb} ]; }}\nwhile ! gd; do t=$(ls -1td */ 2>/dev/null | grep -v -x {e1} | while read d; do for x in target node_modules .venv build dist; do [ -d \"$d$x\" ] && echo \"$d$x\"; done; done | tail -1); [ -z \"$t\" ] && break; echo \"rm $t\"; rm -rf \"$t\"; done\nwhile ! gd; do o=$(ls -1td */ 2>/dev/null | grep -v -x {e2} | tail -1); [ -z \"$o\" ] && break; echo \"rm $o\"; rm -rf \"$o\"; done\nf=$(fr); if [ \"$f\" -lt 5 ]; then echo \"hard $f\"; fi\necho \"free $(fr) GB, office $(us) GB\"",
        b = hb(a, h),
        gb = h.gb,
        e1 = ex(&ac),
        e2 = ex(&ap)
    );
    hx(a, &h.id, &s, 1200).await
}

/// Identifies the Docker daemon used by a configured host. Different host IDs
/// pointing at the same local or SSH Docker target must share one reservation.
fn docker_cache_target(a: &Ax, host: &str) -> Option<String> {
    let configured = crate::sh::ho(a, host)?;
    if !configured.dk {
        return None;
    }
    let ssh = configured.ssh.trim();
    Some(if ssh.is_empty() {
        "docker:local".to_string()
    } else {
        format!("docker:ssh:{ssh}")
    })
}

/// Reserves an idle host for cache maintenance; admission is blocked until Drop.
struct IdleCacheGuard {
    a: Ax,
    target: String,
}

impl IdleCacheGuard {
    fn claim(a: &Ax, host: &str) -> Option<Self> {
        // The same lock serializes admission and cache-cleanup reservation.
        let _transition = a.update_transition.lock().unwrap();
        if a.drain.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        let target = docker_cache_target(a, host)?;
        let active_hosts: Vec<String> = a
            .act
            .lock()
            .unwrap()
            .iter()
            .filter_map(|claim| claim.rsplit_once(' ').map(|(_, h)| h.to_string()))
            .collect();
        if active_hosts
            .iter()
            .any(|active_host| docker_cache_target(a, active_host).as_deref() == Some(&target))
        {
            return None;
        }
        if !a.cache_maintenance.lock().unwrap().insert(target.clone()) {
            return None;
        }
        Some(Self {
            a: a.clone(),
            target,
        })
    }
}

impl Drop for IdleCacheGuard {
    fn drop(&mut self) {
        self.a
            .cache_maintenance
            .lock()
            .unwrap()
            .remove(&self.target);
    }
}

/// Clears an oversized npm download cache only while no work can enter the host.
async fn clean_idle_npm_cache(a: &Ax, host: &crate::sh::Ho) {
    if !host.dk {
        return;
    }
    let Some(_reservation) = IdleCacheGuard::claim(a, &host.id) else {
        return;
    };
    let result =
        crate::sh::hx_maintenance(a, &host.id, &crate::sh::npm_cache_cleanup_cmd(), 600).await;
    if result.c == 0 && result.o.contains("npm cache cleaned") {
        a.say(
            "ci",
            &format!("[{}] npm cache above 4 GiB cleaned while idle", host.id),
            None,
        );
    } else if result.c != 0 {
        a.say(
            "ci",
            &format!(
                "[{}] npm cache maintenance skipped (exit {})",
                host.id, result.c
            ),
            None,
        );
    }
}

pub async fn guard(a: Ax) {
    loop {
        for h in hosts(&a) {
            if !h.on || !a.net(&h.id) {
                continue;
            }
            // Reclaim npm caches first; disk() may remove inactive worktrees
            // when an oversized cache is the actual cause of low free space.
            clean_idle_npm_cache(&a, &h).await;
            let r = disk(&a, &h).await;
            if r.c != 0 {
                continue;
            }
            let rm: Vec<&str> = r.o.lines().filter_map(|l| l.strip_prefix("rm ")).collect();
            let tl = r.o.trim().lines().last().unwrap_or("").to_string();
            if !rm.is_empty() {
                a.say(
                    "ci",
                    &format!("[{}] disk guard freed {} · {tl}", h.id, rm.join(", ")),
                    None,
                );
            }
            if r.o.lines().any(|l| l.starts_with("hard ")) {
                a.note(
                    "Disk almost full",
                    &format!("{}: under 5 GB free · {tl}", h.id),
                    4,
                );
            }
        }
        sl(300_000).await;
    }
}
