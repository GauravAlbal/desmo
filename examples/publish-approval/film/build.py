#!/usr/bin/env python3
"""Render the pinned observed trace into a deterministic local report and silent film."""
import hashlib
import json
import shutil
import subprocess
import tempfile
from pathlib import Path
from runtime_gate import default_binary, resolve_tool, verify_runtime

HERE = Path(__file__).resolve().parent
TRACE = HERE / "source.trace.json"
OUT = HERE / "output"
CHROME_DEFAULTS = ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
FFMPEG_DEFAULTS = ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
FFPROBE_DEFAULTS = ["/opt/homebrew/bin/ffprobe", "/usr/local/bin/ffprobe"]
FACTS = [
    ("F1", 2, "Reviewer approves this exact version", "COMMIT"),
    ("F2", 3, "Author edits the document after approval", "COMMIT"),
    ("F3", 4, "Authorization refused: it names a version nobody approved", "REFUSED"),
    ("F4", 5, "Fresh read of the current version", "READ"),
    ("F5", 6, "Approved again, new version", "COMMIT"),
    ("F6", 8, "Publication authorized", "COMMIT"),
    ("F7", 11, "The authorized artifact is published", "COMMIT"),
]


def digest_comparison(refusal):
    annotation = json.loads(refusal.get("filmFlowAnnotation", "{}"))
    if annotation.get("kind") != "digest-comparison":
        raise ValueError("refusal lacks a labelled film-flow digest annotation")
    approved = annotation.get("approvedDigest", "")
    proposed = annotation.get("proposedDigest", "")
    if len(approved) != 64 or len(proposed) != 64 or approved == proposed:
        raise ValueError("film-flow annotation must contain distinct full artifact digests")
    return approved, proposed


def require_trace(data):
    events = data.get("events", [])
    if data.get("schema") != "ikaros.trace.v2" or data.get("product") != "Ikaros":
        raise ValueError("product trace identity must remain unchanged")
    expected = [(2,"commit","approve"),(3,"commit","revise"),(4,"refusal","authorize_publish"),(6,"commit","approve"),(8,"commit","authorize_publish")]
    for index, kind, command in expected:
        if index >= len(events) or (events[index].get("kind"),events[index].get("attempted")) != (kind,command):
            raise ValueError(f"trace event-{index+1} is not {kind} {command}")
    refusal = events[4]
    if refusal.get("intendedState") != "publishing" or "toState" in refusal:
        raise ValueError("refused publication target must never be represented as reached")
    digest_comparison(refusal)
    if (events[5].get("kind"), events[5].get("attempted")) != ("read", "show") or events[6].get("kind") != "commit":
        raise ValueError("canonical read and renewed approval must remain distinct")
    if (events[11].get("kind"), events[11].get("attempted"), events[11].get("toState")) != ("commit", "complete", "published"):
        raise ValueError("corrected path must commit progress to published")
    return events
CONTAINER_GUARD = "const g=e.parentElement.closest('g');const r=g&&g.querySelector('rect')&&g.querySelector('rect').getBBox();if(r&&(b.x<r.x||b.y<r.y||b.x+b.width>r.x+r.width||b.y+b.height>r.y+r.height))bad.push(e.textContent);"


def run_checked(command, label, text=False):
    for attempt in range(2):
        try:
            return subprocess.run(command, check=True, capture_output=True, text=text)
        except subprocess.CalledProcessError as exc:
            if attempt:
                stderr = exc.stderr.decode("utf-8", errors="replace") if isinstance(exc.stderr, bytes) else exc.stderr or ""
                raise RuntimeError(
                    f"{label} failed after one retry; stderr: {stderr.strip()}"
                ) from exc


def tracker_svg(current, blocked=False):
    labels = ["Draft", "Approved", "Authorized", "Published"]
    positions = [120, 450, 780, 1110]
    active = {"draft": 0, "approved": 1, "publishing": 2, "published": 3}.get(current, 0)
    out = f'<g aria-label="Committed lifecycle state" data-active="{labels[active]}">'
    for i, (label, x) in enumerate(zip(labels, positions)):
        color = "#8ed1c6" if i == active else "#53646c"
        node_class = "tracker-on" if i == active else "tracker-off"
        out += f'<circle class="{node_class}" data-node="{label}" cx="{x}" cy="570" r="15" fill="{color}"/><text class="tracker-label" x="{x}" y="610" text-anchor="middle">{label}</text>'
        if i < 3:
            line = "#8ed1c6" if i < active else "#38505d"
            out += f'<line x1="{x+20}" y1="570" x2="{positions[i+1]-20}" y2="570" stroke="{line}" stroke-width="3"/>'
    if blocked:
        x = positions[2]
        out += f'<path d="M{positions[1]+24} 570H{x-24}" stroke="#ff857c" stroke-width="5" stroke-dasharray="8 7"/><circle cx="{x}" cy="570" r="24" fill="none" stroke="#ff857c" stroke-width="3"/><path d="M{x-10} 560L{x+10} 580M{x+10} 560L{x-10} 580" stroke="#ff857c" stroke-width="4"/>'
    return out + "</g>"

SCENE_BODIES = {
    "title": '''<text class="hero" x="96" y="235">One approved artifact.</text><text class="hero" x="96" y="315">One exact authorization.</text><text class="tag read" x="96" y="367">READ</text><text class="trace" x="196" y="367">{event_id}</text><text class="rule" x="96" y="430">Publication authorization must refer to the exact artifact approved.</text>''',
    "end": '''<text class="hero" x="96" y="225">The authorized artifact is published.</text><text class="sub" x="96" y="335">Install the matching release package to reproduce.</text><text class="install" x="96" y="407">gh release download v0.5.0 -R GauravAlbal/desmo</text><text class="install" x="96" y="451">-p 'ik-darwin-arm64.tar.gz'</text><text class="tag commit" x="96" y="505">COMMIT</text><text class="trace" x="196" y="505">{event_id}</text>''',
    "mismatch": '''<g class="version"><rect x="96" y="360" width="500" height="112" rx="12"/><text class="label" x="124" y="400">APPROVED</text><text class="digest" x="124" y="447">{approved}…</text></g><text class="versus" x="620" y="432">≠</text><g class="version"><rect x="684" y="360" width="500" height="112" rx="12"/><text class="label" x="712" y="400">PROPOSED</text><text class="digest" x="712" y="447">{proposed}…</text></g><text class="sub" x="96" y="524">The authorization names an earlier approval.</text>''',
    "approved": '''<g class="document changed"><rect x="96" y="366" width="500" height="134" rx="12"/><text class="label" x="128" y="410">APPROVED VERSION</text><text class="digest" x="128" y="462">{approved}…</text></g><text class="sub" x="660" y="438">Approval names this exact version.</text>''',
    "edit": '''<g class="document"><rect x="96" y="365" width="290" height="152" rx="12"/><text class="label" x="128" y="410">APPROVED COPY</text><text class="sub" x="128" y="462">version A</text></g><path d="M430 440H610" stroke="#8ed1c6" stroke-width="3"/><path d="M594 426L612 440L594 454" fill="none" stroke="#8ed1c6" stroke-width="3"/><g class="document changed"><rect x="660" y="365" width="290" height="152" rx="12"/><text class="label" x="692" y="410">EDITED COPY</text><text class="sub" x="692" y="462">version B</text></g><text class="sub" x="980" y="435">different bytes</text>''',
    "read": '''<g class="document"><rect x="96" y="366" width="500" height="134" rx="12"/><text class="label" x="128" y="410">CURRENT CANONICAL STATE</text><text class="sub" x="128" y="462">Draft · edited version</text></g><text class="small" x="96" y="536">A read observes state. It does not commit a change.</text>''',
    "approve": '''<g class="document changed"><rect x="96" y="366" width="500" height="134" rx="12"/><text class="label" x="128" y="410">NEW APPROVAL</text><text class="digest" x="128" y="462">{proposed}…</text></g><text class="sub" x="660" y="438">Approval now names this version.</text>''',
    "authorized": '''<g class="document changed"><rect x="96" y="366" width="500" height="134" rx="12"/><text class="label" x="128" y="410">AUTHORIZED VERSION</text><text class="digest" x="128" y="462">{proposed}…</text></g><text class="sub" x="660" y="438">The approved version is authorized.</text>''',
    "published": '''<g class="document published"><rect x="96" y="366" width="500" height="134" rx="12"/><text class="label" x="128" y="410">PUBLISHED</text><text class="sub" x="128" y="462">Progress committed</text></g><text class="sub" x="660" y="438">The lifecycle reaches its committed end state.</text>''',
}

SCENE_SPECS = [
    ("title", "", "", "draft", False),
    ("approved", FACTS[0][2], FACTS[0][3], "approved", False),
    ("edit", FACTS[1][2], FACTS[1][3], "draft", False),
    ("mismatch", FACTS[2][2], FACTS[2][3], "draft", True),
    ("read", FACTS[3][2], FACTS[3][3], "draft", False),
    ("approve", FACTS[4][2], FACTS[4][3], "approved", False),
    ("authorized", FACTS[5][2], FACTS[5][3], "publishing", False),
    ("published", FACTS[6][2], FACTS[6][3], "published", False),
    ("end", "", "", "published", False),
]
SOURCE_INDEXES = [0, 2, 3, 4, 5, 6, 8, 11, 11]


def scene_svg(caption, tag, event_id, stage, approved, proposed, state, blocked=False):
    tracker = tracker_svg(state, blocked)
    body = SCENE_BODIES[stage].format(
        event_id=event_id,
        approved=approved[:8],
        proposed=proposed[:8],
    )
    if stage not in ("title", "end"):
        body = f'<text class="caption" x="96" y="231">{caption}</text><text class="tag {tag.lower()}" x="96" y="302">{tag}</text><text class="trace" x="196" y="302">{event_id}</text>' + body
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720" viewBox="0 0 1280 720"><rect width="1280" height="720" fill="#101820"/><style>text{{font-family:Arial,sans-serif}}.hero{{font-size:55px;font-weight:700;fill:#f4f7f8}}.rule{{font-size:29px;font-weight:600;fill:#f2c66d}}.caption{{font-size:34px;font-weight:700;fill:#f4f7f8}}.sub{{font-size:25px;fill:#c5d1d5}}.tag{{font-size:17px;font-weight:700;letter-spacing:1.5px}}.commit{{fill:#8ed1c6}}.read{{fill:#a7c4e8}}.refused{{fill:#ff857c}}.trace,.small{{font-size:16px;fill:#9cadb4}}.label{{font-size:17px;font-weight:700;letter-spacing:1px;fill:#b5c3c8}}.digest{{font:700 29px monospace;fill:#f4f7f8}}.versus{{font-size:45px;font-weight:700;fill:#ff857c}}.version rect,.document rect{{fill:#1a2b35;stroke:#38505d;stroke-width:2}}.changed rect{{stroke:#8ed1c6}}.published rect{{stroke:#8ed1c6;stroke-width:3}}.install{{font-size:20px;font-weight:600;fill:#f2c66d}}.tracker-label{{font-size:16px;font-weight:600;fill:#c5d1d5}}</style><text x="96" y="72" font-size="18" font-weight="700" letter-spacing="2" fill="#8ed1c6">DESMO / EXACT-ARTIFACT APPROVAL</text>{tracker}{body}<script><![CDATA[(()=>{{const s=document.documentElement;let bad=[];for(const e of s.querySelectorAll('text')){{const b=e.getBBox();if(b.x < 0 || b.y < 0 || b.x+b.width > 1220 || b.y+b.height > 690)bad.push(e.textContent)}}s.setAttribute('data-overflow',bad.length?'true':'false');s.setAttribute('data-overflow-items',bad.join('|'));}})()]]></script></svg>'''


def run_chrome(svg_path, screenshot=None, scene="scene"):
    chrome = resolve_tool("IK_CHROME", "google-chrome", CHROME_DEFAULTS)
    command = [chrome, "--headless=new", "--no-sandbox", "--no-first-run"]
    command += [f"--screenshot={screenshot}", "--window-size=1280,720"] if screenshot else ["--dump-dom"]
    command.append(svg_path.as_uri())
    return run_checked(command, f"Chrome scene {scene}", text=True).stdout


def check_refusal_video_frames(movie):
    ffmpeg = resolve_tool("IK_FFMPEG", "ffmpeg", FFMPEG_DEFAULTS)
    vf = "select='between(t,9.43,16.85)',crop=2:2:780:560,format=rgb24"
    result = run_checked(
        [ffmpeg, "-v", "error", "-i", str(movie), "-vf", vf,
         "-vsync", "0", "-f", "rawvideo", "-"],
        "ffmpeg refusal/read frame check",
    ).stdout
    if not result or len(result) % 12:
        raise ValueError("could not sample all refused/read frame tracker pixels")
    frames = len(result) // 12
    for offset in range(0, len(result), 3):
        red, green, blue = result[offset:offset+3]
        if green > red + 35 and green > blue + 20:
            raise ValueError("Authorized node is lit in a refused/read video frame")
    return frames


def fact_record(fact_id, event_index, event_id, event, statement):
    return {
        "id": fact_id,
        "trace_event_id": event_id,
        "event_index": event_index,
        "kind": event["kind"],
        "command": event["attempted"],
        "on_screen_fact": statement,
    }


def render_one_scene(tmp, number, spec, events, approved, proposed):
    stage, caption, tag, state, blocked = spec
    event_index = SOURCE_INDEXES[number]
    event_id = f"event-{event_index+1}"
    svg = scene_svg(caption, tag, event_id, stage, approved, proposed, state, blocked)
    svg = svg.replace("const b=e.getBBox();", "const b=e.getBBox();" + CONTAINER_GUARD)
    svg_path = tmp / f"scene-{number}.svg"
    svg_path.write_text(svg)
    dom = run_chrome(svg_path, scene=stage)
    if 'data-overflow="false"' not in dom:
        start = dom.find("data-overflow")
        raise ValueError(f"scene {stage} overflows text bounds: {dom[start:start+240]}")
    if number in (3, 4) and (
        'data-active="Draft"' not in dom
        or 'class="tracker-off" data-node="Authorized"' not in dom
    ):
        raise ValueError(f"frame {stage} lights Authorized before renewed approval")
    if number == 0:
        event = events[event_index]
        fact = fact_record("F0", event_index, event_id, event, "Opening state is draft")
        run_chrome(svg_path, tmp / "scene-0.png", scene=stage)
        return None, fact
    if number in range(1, 8):
        png = tmp / f"scene-{number}.png"
        run_chrome(svg_path, png, scene=stage)
        event = events[event_index]
        fact = fact_record(FACTS[number-1][0], event_index, event_id, event, caption)
        return png, fact
    return None, None


def render_story_scenes(tmp, events, approved, proposed):
    frames, facts = [], []
    for number, spec in enumerate(SCENE_SPECS):
        frame, fact = render_one_scene(tmp, number, spec, events, approved, proposed)
        if frame is not None:
            frames.append(frame)
        if fact is not None:
            facts.append(fact)
    if len(frames) != 7:
        raise ValueError("each narrative scene must have a rendered frame")
    for number, png in enumerate(frames[:6], 1):
        shutil.copyfile(png, OUT / f"still-{number}.png")
    shutil.copyfile(tmp / "scene-0.png", OUT / "poster.png")
    return frames, facts


def render_release_card(tmp, approved, proposed):
    end_path = tmp / "scene-8.svg"
    svg = scene_svg("", "", f"event-{SOURCE_INDEXES[8]+1}", "end", approved, proposed, "published")
    end_path.write_text(svg.replace("const b=e.getBBox();", "const b=e.getBBox();" + CONTAINER_GUARD))
    end_dom = run_chrome(end_path, scene="end card")
    if 'data-overflow="false"' not in end_dom:
        raise ValueError("release card text overflows")
    frame = tmp / "scene-8.png"
    run_chrome(end_path, frame, scene="end card")
    return frame


def encode_movie(tmp, frames, end_frame):
    sequence = [tmp / "scene-0.png"] + frames + [end_frame]
    durations = [2.0] + [26/7] * 7 + [2.0]
    concat = tmp / "frames.txt"
    concat.write_text("".join(f"file '{path}'\nduration {duration:.9f}\n" for path, duration in zip(sequence, durations)) + f"file '{sequence[-1]}'\n")
    movie = OUT / "approval-film.mp4"
    ffmpeg = resolve_tool("IK_FFMPEG", "ffmpeg", FFMPEG_DEFAULTS)
    run_checked(
        [ffmpeg, "-y", "-f", "concat", "-safe", "0", "-i", str(concat), "-vf", "fps=30,format=yuv420p", "-t", "30", "-an", "-movflags", "+faststart", str(movie)],
        "ffmpeg encode approval film",
    )
    return movie, check_refusal_video_frames(movie)


def render_outputs(events, approved, proposed):
    OUT.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        tmp = Path(temporary)
        frames, facts = render_story_scenes(tmp, events, approved, proposed)
        end_frame = render_release_card(tmp, approved, proposed)
        movie, sampled_frames = encode_movie(tmp, frames, end_frame)
    return facts, movie, sampled_frames


def write_report(events, approved, proposed, facts):
    (OUT / "facts.json").write_text(json.dumps(facts, indent=2) + "\n")
    refusal = events[4]
    report = '''<!doctype html><meta charset="utf-8"><title>Exact-artifact approval</title><style>body{font:18px system-ui;max-width:900px;margin:3rem auto;background:#101820;color:#eef}li{margin:1rem}code{color:#8ed1c6}</style><h1>Exact-artifact approval</h1><p><b>Publication authorization must refer to the exact artifact that was approved.</b></p><p>A reasonable edit changes the approved artifact. Authorization using the old approval is refused; the current state is read canonically, the edited artifact is approved, and progress then commits successfully.</p><p>Observed refusal (event-5): ''' + refusal["code"] + '''; approved digest <code>''' + approved + '''</code>, proposed digest <code>''' + proposed + '''</code>. Intended state <code>''' + refusal["intendedState"] + '''</code> was refused, not reached.</p><ol>''' + ''.join(f"<li>{fact['id']} — {fact['on_screen_fact']} (<code>{fact['trace_event_id']}</code>: {fact['kind']} {fact['command']})</li>" for fact in facts) + '''</ol><p>Reads and historical receipt replay are not commits. No upload, external publisher, human evidence, or attestation claim.</p><h2>Silent-comprehension questionnaire</h2><ol><li>What rule is protected?</li><li>What proposal failed?</li><li>What is the correct next action?</li></ol><p>Human run: open owner item; not performed.</p>'''
    (OUT / "report.html").write_text(report)


def write_manifest(data, facts, movie, sampled_frames):
    machine = HERE.parent / "machines" / "publish_approval_v3.ik.toml"
    digest = hashlib.sha256(movie.read_bytes()).hexdigest()
    ffprobe = resolve_tool("IK_FFPROBE", "ffprobe", FFPROBE_DEFAULTS)
    duration = run_checked(
        [ffprobe, "-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1", str(movie)],
        "ffprobe film duration",
        text=True,
    ).stdout.strip()
    manifest = {"source_trace": "../source.trace.json", "source_trace_sha256": hashlib.sha256(TRACE.read_bytes()).hexdigest(), "runtime_version": data["runtime_version"], "machine_id": data["machine_id"], "machine_definition": str(machine.relative_to(HERE.parent.parent.parent)), "machine_sha256": hashlib.sha256(machine.read_bytes()).hexdigest(), "schema": data["schema"], "duration_seconds": duration, "audio": "none", "mp4_bytes": movie.stat().st_size, "mp4_sha256": digest, "outputs": sorted({path.name for path in OUT.iterdir()} | {"validation.log"}), "facts": facts, "overflow": "all rendered scenes passed viewport and available text-container bounds checks", "refused_read_video_frames_checked": sampled_frames, "human_run": "open owner item; not performed"}
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"duration": duration, "bytes": movie.stat().st_size, "sha256": digest, "facts": len(facts), "overflow": "passed", "frame_state": "Authorized unlit through refusal/read interval"}, indent=2))


def main():
    data = json.loads(TRACE.read_text())
    verify_runtime(default_binary(), data["runtime_version"])
    events = require_trace(data)
    approved, proposed = digest_comparison(events[4])
    facts, movie, sampled_frames = render_outputs(events, approved, proposed)
    write_report(events, approved, proposed, facts)
    import checks
    checks.main()
    checks.record_render_checks(9, sampled_frames)
    write_manifest(data, facts, movie, sampled_frames)

if __name__ == "__main__":
    main()
