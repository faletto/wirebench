"""Generate a large lab for benchmarks: a chain of N routers, one PC per router, with full
static routing so a ping from the first PC to the last crosses every router.

    python3 testkit/bench/gen_chain.py 50 > /tmp/chain-100.lab.toml

N routers + N PCs = 2N devices, (N-1) + N links. Output is deterministic.
"""
import sys


def main(n: int) -> None:
    out = ['format  = "labsim-lab"', "version = 1", "", "[meta]",
           f'title  = "Chain of {n} routers, {n} PCs"', "seed   = 1", ""]
    for i in range(n):
        cfg = [f"hostname r{i}", "!"]
        if i > 0:
            cfg += ["interface GigabitEthernet0/0", f" ip address 10.1.{i - 1}.2 255.255.255.252", " no shutdown", "!"]
        if i < n - 1:
            cfg += ["interface GigabitEthernet0/1", f" ip address 10.1.{i}.1 255.255.255.252", " no shutdown", "!"]
        cfg += ["interface GigabitEthernet0/2", f" ip address 10.2.{i}.1 255.255.255.0", " no shutdown", "!"]
        for j in range(n):
            if j == i:
                continue
            via = f"10.1.{i}.2" if j > i else f"10.1.{i - 1}.1"
            cfg.append(f"ip route 10.2.{j}.0 255.255.255.0 {via}")
        out += ["[[device]]", f'name     = "r{i}"', 'template = "router-3g"',
                f"pos      = [{i * 120}, 100]", 'startup_config = """', *cfg, '"""', ""]
    for i in range(n):
        out += ["[[device]]", f'name     = "h{i}"', 'template = "host-pc"',
                f"pos      = [{i * 120}, 260]", "[device.host]",
                f'ipv4    = "10.2.{i}.2/24"', f'gateway = "10.2.{i}.1"', ""]
    for i in range(n - 1):
        out += ["[[link]]", f'a = "r{i}:Gi0/1"', f'b = "r{i + 1}:Gi0/0"', ""]
    for i in range(n):
        out += ["[[link]]", f'a = "h{i}:eth0"', f'b = "r{i}:Gi0/2"', ""]
    sys.stdout.write("\n".join(out))


if __name__ == "__main__":
    main(int(sys.argv[1]) if len(sys.argv) > 1 else 50)
