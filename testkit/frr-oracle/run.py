#!/usr/bin/env python3
"""Run FRR's ospfd (in Docker) on the networks in cases/*.case and record what it computes.

    testkit/frr-oracle/run.py [case ...]     # default: every case; writes cases/<name>.expected

Needs Docker and the image below. The case format is described in README.md. Each router is a
container, each segment a Docker network (10.<area>.<segment>.0/24, member k at .k+1, the
network's own gateway at .254), and the answer is `show ip ospf route json` after convergence,
reduced to `node<TAB>prefix<TAB>kind<TAB>cost` lines.
"""
import json, pathlib, subprocess, sys, time

IMAGE = "quay.io/frrouting/frr:10.2.1"
HERE = pathlib.Path(__file__).resolve().parent
DUMP_IMAGE = "wb-tcpdump"  # built below from alpine plus tcpdump
CAP = HERE / "captures"
SETTLE = 120  # seconds; DR election waits 40 and the database exchange takes a few more


def sh(*a, check=True):
    r = subprocess.run(a, capture_output=True, text=True)
    if check and r.returncode:
        sys.exit(f"{' '.join(a)}\n{r.stdout}{r.stderr}")
    return r.stdout


def parse(path):
    nodes, segs, default = 0, [], {}
    for line in path.read_text().splitlines():
        line = line.split("#")[0].split()
        if not line:
            continue
        if line[0] == "nodes":
            nodes = int(line[1])
        elif line[0] == "seg":
            members = [tuple(int(x) for x in m.split("/")) for m in line[4:]]
            segs.append((line[1] == "p2p", int(line[2]), members))
        elif line[0] == "default":
            default[int(line[1])] = (int(line[3]), line[2] == "e2")
    return nodes, segs, default


def run_case(path):
    name = path.stem
    nodes, segs, default = parse(path)
    cname = lambda n: f"wb-{name}-r{n}"
    nname = lambda s: f"wb-{name}-s{s}"
    addr = lambda s, k: f"10.{segs[s][1]}.{s}.{k + 1}"
    ifs = {n: [] for n in range(nodes)}  # node -> [(seg, k, prio, cost)]
    for s, (_, _, members) in enumerate(segs):
        for k, (node, prio, cost) in enumerate(members):
            ifs[node].append((s, k, prio, cost))
    cleanup(name)
    CAP.mkdir(exist_ok=True)
    if not sh("docker", "images", "-q", DUMP_IMAGE).strip():
        subprocess.run(["docker", "build", "-q", "-t", DUMP_IMAGE, "-f", "-", "."], input=
                       "FROM alpine:3.20\nRUN apk add --no-cache tcpdump\nENTRYPOINT [\"tcpdump\"]\n",
                       text=True, check=True, cwd=HERE)
    try:
        for s, (_, area, _) in enumerate(segs):
            sh("docker", "network", "create", "--internal", "--subnet", f"10.{area}.{s}.0/24",
               "--gateway", f"10.{area}.{s}.254", nname(s))
        for n in range(nodes):
            first = ifs[n][0]
            sh("docker", "create", "--name", cname(n), "--network", nname(first[0]),
               "--ip", addr(first[0], first[1]), "--cap-add", "NET_ADMIN", "--cap-add", "SYS_ADMIN",
               "--entrypoint", "sh", IMAGE, "-c",
               "sed -i 's/^ospfd=no/ospfd=yes/' /etc/frr/daemons; touch /etc/frr/frr.conf; "
               "chown frr:frr /etc/frr/frr.conf; exec /sbin/tini -- /usr/lib/frr/docker-start")
            for s, k, _, _ in ifs[n][1:]:
                sh("docker", "network", "connect", "--ip", addr(s, k), nname(s), cname(n))
        for n in range(nodes):
            sh("docker", "start", cname(n))
        for n in range(nodes):
            for _ in range(60):
                if "ospfd" in sh("docker", "exec", cname(n), "sh", "-c", "ls /var/run/frr", check=False):
                    break
                time.sleep(1)
            time.sleep(1)
            # Record every OSPF packet this router sends and hears (see captures/ and the README).
            sh("docker", "run", "-d", "--name", f"{cname(n)}-dump", "--net", f"container:{cname(n)}",
               "--cap-add", "NET_RAW", "--cap-add", "NET_ADMIN", "-v", f"{CAP}:/cap", DUMP_IMAGE,
               "-i", "any", "-U", "-w", f"/cap/{name}.r{n}.pcap", "ip proto 89")
            time.sleep(1)
            links = {}
            for line in sh("docker", "exec", cname(n), "ip", "-o", "-4", "addr").splitlines():
                f = line.split()
                links[f[3].split("/")[0]] = f[1]
            conf = []
            conf.append(f"router ospf")
            rid = max((addr(s, k) for s, k, _, _ in ifs[n]), key=lambda a: tuple(map(int, a.split("."))))
            conf.append(f" ospf router-id {rid}")
            for s, k, _, _ in ifs[n]:
                conf.append(f" network 10.{segs[s][1]}.{s}.0/24 area {segs[s][1]}")
            if n in default:
                metric, e2 = default[n]
                conf.append(f" default-information originate always metric {metric} metric-type {2 if e2 else 1}")
            conf.append("exit")
            for s, k, prio, cost in ifs[n]:
                conf += [f"interface {links[addr(s, k)]}", f" ip ospf cost {cost}",
                         f" ip ospf priority {prio}"]
                if segs[s][0]:
                    conf.append(" ip ospf network point-to-point")
                conf.append("exit")
            f = HERE / f".{name}.{n}.conf"
            f.write_text("\n".join(conf) + "\n")
            sh("docker", "cp", str(f), f"{cname(n)}:/tmp/c.conf")
            f.unlink()
            sh("docker", "exec", cname(n), "vtysh", "-f", "/tmp/c.conf")
        time.sleep(SETTLE)
        for n in range(nodes):
            sh("docker", "stop", "-t", "2", f"{cname(n)}-dump")
        out = []
        for n in range(nodes):
            routes = json.loads(sh("docker", "exec", cname(n), "vtysh", "-c", "show ip ospf route json"))
            for prefix, r in routes.items():
                if "/" not in prefix:
                    continue  # router entries (ABR / ASBR)
                kind = {"N": "intra", "N IA": "inter", "N E1": "e1", "N E2": "e2"}[r["routeType"]]
                cost = r["type2cost"] if kind == "e2" else r["cost"]
                out.append(f"{n}\t{prefix}\t{kind}\t{cost}")
        out.sort(key=lambda l: (int(l.split("\t")[0]), l))
        (HERE / "cases" / f"{name}.expected").write_text("\n".join(out) + "\n")
        print(f"{name}: {len(out)} routes")
    finally:
        cleanup(name)


def cleanup(name):
    for c in sh("docker", "ps", "-aq", "--filter", f"name=wb-{name}-").split():
        sh("docker", "rm", "-f", c, check=False)
    for n in sh("docker", "network", "ls", "-q", "--filter", f"name=wb-{name}-").split():
        sh("docker", "network", "rm", n, check=False)


if __name__ == "__main__":
    cases = [HERE / "cases" / f"{a}.case" for a in sys.argv[1:]] or sorted((HERE / "cases").glob("*.case"))
    for c in cases:
        run_case(c)
