#!/usr/bin/env python3
"""Third-party inventory, licence texts and SPDX 2.3 SBOM (docs/LICENSING.md).

Collects what actually ships:
  - Rust crates reachable from the workspace binaries (cargo metadata, all
    features, locked);
  - npm production dependencies of the Studio bundle (apps/studio) and the
    phone app (apps/mobile);
  - Ruby gems of the control plane (Gemfile.lock, versions; licences from
    the installed gemspecs when `--gems-from-bundle` is given);
  - optionally Debian/Ubuntu packages of an OS image (`--dpkg <file>` with
    `dpkg-query -W -f='${Package}\t${Version}\t${source:Package}\n'`).

Writes <out>/sbom.spdx.json, <out>/THIRD_PARTY.md and <out>/LICENSES/...,
and fails if any shipped component has no licence or one outside POLICY.
Usage: ci/sbom.py <out-dir> [--scope all|device] [--gems-from-bundle] [--dpkg FILE] [--version V]
  --scope device  what the OS image ships (no phone app)
"""
import datetime, hashlib, json, os, re, shutil, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LICENSE_FILES = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|COPYRIGHT|UNLICENSE)([-._].*)?$", re.I)
# Licences acceptable in the shipped product. GPL/LGPL appear only through
# system libraries (x264, FFmpeg, kernel, BlueZ, ...) whose obligations
# docs/LICENSING.md spells out; they are allowed for dpkg components only.
PERMISSIVE = {"MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib", "0BSD",
              "Unlicense", "Unicode-3.0", "Unicode-DFS-2016", "CDLA-Permissive-2.0", "BlueOak-1.0.0", "CC0-1.0", "Ruby", "BSD-2-Clause-Views",
              "Python-2.0", "MPL-2.0", "CC-BY-4.0"}


def run(*cmd, cwd=ROOT):
    return subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True).stdout


def spdx_ok(expr):
    """True if some alternative of an SPDX expression is entirely permissive."""
    if not expr:
        return False
    expr = expr.replace("/", " OR ")
    for alt in re.split(r"\s+OR\s+", expr.replace("(", "").replace(")", "")):
        parts = [p.strip() for p in re.split(r"\s+AND\s+", alt)]
        if all(p in PERMISSIVE for p in parts):
            return True
    return False


def copy_texts(src_dir, dest):
    copied = []
    if not src_dir or not os.path.isdir(src_dir):
        return copied
    for f in sorted(os.listdir(src_dir)):
        p = os.path.join(src_dir, f)
        if LICENSE_FILES.match(f) and os.path.isfile(p):
            os.makedirs(dest, exist_ok=True)
            shutil.copyfile(p, os.path.join(dest, f))
            copied.append(f)
    return copied


def rust(out):
    meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--all-features", "--locked"))
    members = set(meta["workspace_members"])
    by_id = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, stack = set(), list(members)
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            # Build-only dependencies do not ship.
            if any(k.get("kind") in (None, "normal") for k in d["dep_kinds"]):
                stack.append(d["pkg"])
    comps = []
    for i in sorted(seen - members, key=lambda x: (by_id[x]["name"], by_id[x]["version"])):
        p = by_id[i]
        dest = os.path.join(out, "LICENSES", "rust", f"{p['name']}-{p['version']}")
        texts = copy_texts(os.path.dirname(p["manifest_path"]), dest)
        comps.append({"ecosystem": "cargo", "name": p["name"], "version": p["version"], "license": p.get("license") or "",
                      "purl": f"pkg:cargo/{p['name']}@{p['version']}", "texts": texts, "source": p.get("source") or ""})
    return comps


def npm(out, app):
    cwd = os.path.join(ROOT, app)
    if not os.path.isdir(os.path.join(cwd, "node_modules")):
        raise SystemExit(f"{app}: run npm ci first")
    tree = json.loads(subprocess.run(["npm", "ls", "--omit=dev", "--all", "--json", "--long"], cwd=cwd, capture_output=True, text=True).stdout or "{}")
    found = {}

    def walk(deps):
        for name, d in (deps or {}).items():
            if "version" in d and (name, d["version"]) not in found:
                found[(name, d["version"])] = d.get("path") or os.path.join(cwd, "node_modules", name)
            walk(d.get("dependencies"))
    walk(tree.get("dependencies"))
    comps = []
    for (name, version), path in sorted(found.items()):
        pkg = {}
        try:
            pkg = json.load(open(os.path.join(path, "package.json")))
        except OSError:
            pass
        lic = pkg.get("license") or ""
        if isinstance(lic, dict):
            lic = lic.get("type", "")
        if not lic and isinstance(pkg.get("licenses"), list):
            lic = " OR ".join(x.get("type", "") for x in pkg["licenses"] if isinstance(x, dict))
        dest = os.path.join(out, "LICENSES", "npm", app.replace("/", "_"), f"{name.replace('/', '__')}-{version}")
        comps.append({"ecosystem": "npm", "name": name, "version": version, "license": lic, "app": app,
                      "purl": f"pkg:npm/{name.replace('@', '%40')}@{version}", "texts": copy_texts(path, dest)})
    return comps


def gems(out, from_bundle):
    lock = open(os.path.join(ROOT, "services/control/Gemfile.lock")).read()
    specs = re.findall(r"^    ([a-z0-9_.-]+) \(([^)\s]+)\)$", lock.split("PLATFORMS")[0], re.M)
    licenses, dirs = {}, {}
    if from_bundle:
        script = 'Bundler.load.specs.each { |s| puts [s.name, s.version, s.licenses.join(" OR "), s.full_gem_path].join("\\t") }'
        for line in run("bundle", "exec", "ruby", "-e", script, cwd=os.path.join(ROOT, "services/control")).splitlines():
            n, v, l, d = line.split("\t")
            licenses[(n, v)], dirs[(n, v)] = l, d
    comps = []
    for name, version in sorted(set(specs)):
        version = version.split("-")[0] if (name, version) not in licenses else version
        lic = licenses.get((name, version), "")
        dest = os.path.join(out, "LICENSES", "gem", f"{name}-{version}")
        comps.append({"ecosystem": "gem", "name": name, "version": version, "license": lic,
                      "purl": f"pkg:gem/{name}@{version}", "texts": copy_texts(dirs.get((name, version)), dest)})
    return comps


def dpkg(path):
    comps = []
    for line in open(path):
        parts = line.rstrip("\n").split("\t")
        if len(parts) >= 2 and parts[0]:
            src = parts[2] if len(parts) > 2 and parts[2] else parts[0]
            comps.append({"ecosystem": "deb", "name": parts[0], "version": parts[1], "license": "", "source_package": src,
                          "purl": f"pkg:deb/ubuntu/{parts[0]}@{parts[1]}", "texts": []})
    return comps


def spdx_id(c):
    return "SPDXRef-" + re.sub(r"[^A-Za-z0-9.-]", "-", f"{c['ecosystem']}-{c['name']}-{c['version']}")


def main():
    args = sys.argv[1:]
    if not args:
        raise SystemExit(__doc__)
    out = os.path.abspath(args[0])
    version = args[args.index("--version") + 1] if "--version" in args else "dev"
    if os.path.isdir(out):
        shutil.rmtree(out)
    os.makedirs(out)
    scope = args[args.index("--scope") + 1] if "--scope" in args else "all"
    if scope not in ("all", "device"):
        raise SystemExit("--scope must be all or device")
    comps = rust(out) + npm(out, "apps/studio") + (npm(out, "apps/mobile") if scope == "all" else []) + gems(out, "--gems-from-bundle" in args)
    if "--dpkg" in args:
        comps += dpkg(args[args.index("--dpkg") + 1])
    problems = []
    for c in comps:
        if c["ecosystem"] == "deb":
            continue  # licences from /usr/share/doc/*/copyright, see docs/LICENSING.md
        if c["ecosystem"] == "gem" and "--gems-from-bundle" not in args:
            continue
        if not spdx_ok(c["license"]):
            problems.append(f"{c['ecosystem']} {c['name']} {c['version']}: licence {c['license']!r} not in policy")
    now = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch:
        now = datetime.datetime.fromtimestamp(int(epoch), datetime.timezone.utc).isoformat().replace("+00:00", "Z")
    doc = {
        "spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"t3box-{version}",
        "documentNamespace": f"https://github.com/ryosk7/T3Box/sbom/{version}/{hashlib.sha256(json.dumps([c['purl'] for c in comps]).encode()).hexdigest()[:16]}",
        "creationInfo": {"created": now, "creators": ["Tool: t3box-ci-sbom"]},
        "packages": [{"name": "t3box", "SPDXID": "SPDXRef-t3box", "versionInfo": version, "downloadLocation": "https://github.com/ryosk7/T3Box",
                      "licenseConcluded": "MIT", "licenseDeclared": "MIT", "filesAnalyzed": False}] + [
            {"name": c["name"], "SPDXID": spdx_id(c), "versionInfo": c["version"], "downloadLocation": "NOASSERTION",
             "licenseConcluded": "NOASSERTION", "licenseDeclared": c["license"] or "NOASSERTION", "filesAnalyzed": False,
             "externalRefs": [{"referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl", "referenceLocator": c["purl"]}]}
            for c in comps],
        "relationships": [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": "SPDXRef-t3box"}] + [
            {"spdxElementId": "SPDXRef-t3box", "relationshipType": "CONTAINS", "relatedSpdxElement": spdx_id(c)} for c in comps],
    }
    json.dump(doc, open(os.path.join(out, "sbom.spdx.json"), "w"), indent=1, sort_keys=True)
    counts = {}
    for c in comps:
        counts[c["ecosystem"]] = counts.get(c["ecosystem"], 0) + 1
    with open(os.path.join(out, "THIRD_PARTY.md"), "w") as f:
        f.write(f"# Third-party components of T3Box {version}\n\nGenerated by ci/sbom.py. Licence texts: LICENSES/. Obligations: docs/LICENSING.md.\n\n")
        f.write("| Ecosystem | Component | Version | Licence |\n|---|---|---|---|\n")
        for c in comps:
            f.write(f"| {c['ecosystem']} | {c['name']} | {c['version']} | {c['license'] or 'see copyright file'} |\n")
    without_text = sum(1 for c in comps if c["ecosystem"] != "deb" and not c["texts"])
    print(json.dumps({"components": counts, "problems": len(problems), "without_licence_file": without_text}))
    for p in problems:
        print("POLICY", p)
    if problems:
        sys.exit(1)


if __name__ == "__main__":
    main()
