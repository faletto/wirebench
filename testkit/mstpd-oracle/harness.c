/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * mstpd as a spanning tree oracle: mstpd's protocol core (mstp.c) driven by a script, with no
 * kernel, sockets or clock. Bridges are joined by point to point links that deliver each BPDU at
 * once, in order, and every bridge is given one second at a time, the same model as
 * crates/netdev/tests/stp_oracle.rs uses for our own machine. The two print the same report, so
 * the test compares text.
 *
 * This file is GPL because it is linked with mstp.c. It is built only for tests and never shipped
 * (see README.md).
 *
 * Script, one command per line ('#' starts a comment):
 *   mode stp|rstp                      before any bridge
 *   bridge <priority> <mac, 12 hex digits> <ports>
 *                                      bridges are numbered from 0, ports from 1
 *   cost <bridge> <port> <cost>
 *   portprio <bridge> <port> <priority>
 *   link <bridge> <port> <bridge> <port>
 *                                      links are numbered from 0 and start down
 *   up <link> | down <link>
 *   run <seconds>
 *   dump
 */
#include <ctype.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <linux/if_bridge.h>
#include <asm/byteorder.h>

#include "mstp.h"
#include "log.h"

int log_level = LOG_LEVEL_ERROR;

void vDprintf(int level, const char *fmt, va_list ap)
{
    if(level > log_level)
        return;
    vfprintf(stderr, fmt, ap);
    fputc('\n', stderr);
}

void Dprintf(int level, const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vDprintf(level, fmt, ap);
    va_end(ap);
}

#define MAX_BRIDGES 64
#define MAX_PORTS 64
#define MAX_LINKS 512
#define MAX_QUEUE 65536

typedef struct
{
    bridge_t br;
    int n_ports;
    port_t *ports[MAX_PORTS + 1]; /* indexed by port number */
} sim_bridge_t;

typedef struct
{
    int b[2], p[2];
    bool up;
} sim_link_t;

typedef struct
{
    port_t *from;
    int size;
    unsigned char data[sizeof(bpdu_t)];
} queued_t;

static sim_bridge_t bridges[MAX_BRIDGES];
static int n_bridges;
static sim_link_t links[MAX_LINKS];
static int n_links;
static protocol_version_t mode = protoRSTP;
static queued_t *queue;
static int q_head, q_tail;
static long seconds;
static int line_no;

static void die(const char *fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    fprintf(stderr, "mstpd-oracle: line %d: ", line_no);
    vfprintf(stderr, fmt, ap);
    fputc('\n', stderr);
    va_end(ap);
    exit(2);
}

/* ---- what mstp.c asks of its driver ---- */

void MSTP_OUT_set_state(per_tree_port_t *ptp, int new_state)
{
    ptp->state = new_state;
}

void MSTP_OUT_flush_all_fids(per_tree_port_t *ptp)
{
    MSTP_IN_all_fids_flushed(ptp);
}

void MSTP_OUT_set_ageing_time(port_t *prt, unsigned int ageingTime)
{
    (void)prt;
    (void)ageingTime;
}

void MSTP_OUT_tx_bpdu(port_t *prt, bpdu_t *bpdu, int size)
{
    if(q_tail - q_head >= MAX_QUEUE)
        die("BPDUs never stop");
    if(size < 0 || size > (int)sizeof(bpdu_t))
        die("BPDU of %d bytes", size);
    queued_t *q = &queue[q_tail++ % MAX_QUEUE];
    q->from = prt;
    q->size = size;
    memcpy(q->data, bpdu, size);
}

void MSTP_OUT_shutdown_port(port_t *prt)
{
    (void)prt;
}

/* ---- the network ---- */

static port_t *peer(port_t *prt)
{
    for(int l = 0; l < n_links; ++l)
    {
        if(!links[l].up)
            continue;
        for(int e = 0; e < 2; ++e)
        {
            sim_bridge_t *b = &bridges[links[l].b[e]];
            if(b->ports[links[l].p[e]] == prt)
            {
                sim_bridge_t *o = &bridges[links[l].b[1 - e]];
                return o->ports[links[l].p[1 - e]];
            }
        }
    }
    return NULL;
}

static void drain(void)
{
    long steps = 0;
    while(q_head != q_tail)
    {
        queued_t q = queue[q_head++ % MAX_QUEUE];
        port_t *to = peer(q.from);
        if(!to || !to->portEnabled)
            continue;
        if(++steps > 1000000)
            die("BPDUs never stop");
        bpdu_t bpdu;
        memset(&bpdu, 0, sizeof(bpdu));
        memcpy(&bpdu, q.data, q.size);
        MSTP_IN_rx_bpdu(to, &bpdu, q.size);
    }
}

static port_t *port_of(int b, int p)
{
    if(b < 0 || b >= n_bridges)
        die("no bridge %d", b);
    if(p < 1 || p > bridges[b].n_ports)
        die("bridge %d has no port %d", b, p);
    return bridges[b].ports[p];
}

static void set_link(int l, bool up)
{
    if(l < 0 || l >= n_links)
        die("no link %d", l);
    links[l].up = up;
    for(int e = 0; e < 2; ++e)
    {
        port_t *prt = port_of(links[l].b[e], links[l].p[e]);
        prt->sysdeps.up = up;
        MSTP_IN_set_port_enable(prt, up, up ? 100 : 0, up ? 1 : 0);
        drain();
    }
}

static void add_bridge(unsigned priority, const char *mac, int n_ports)
{
    if(n_bridges == MAX_BRIDGES)
        die("too many bridges");
    if(n_ports < 1 || n_ports > MAX_PORTS)
        die("ports must be 1..%d", MAX_PORTS);
    if(priority % 4096 || priority > 61440)
        die("priority %u is not a multiple of 4096", priority);
    if(strlen(mac) != 12)
        die("mac must be 12 hex digits");
    int i = n_bridges++;
    sim_bridge_t *s = &bridges[i];
    bridge_t *br = &s->br;
    for(int k = 0; k < ETH_ALEN; ++k)
    {
        unsigned v = 0;
        if(!isxdigit((unsigned char)mac[2 * k]) || !isxdigit((unsigned char)mac[2 * k + 1])
           || sscanf(mac + 2 * k, "%2x", &v) != 1)
            die("bad mac %s", mac);
        br->sysdeps.macaddr[k] = (__u8)v;
    }
    snprintf(br->sysdeps.name, IFNAMSIZ, "b%d", i);
    br->sysdeps.if_index = i + 1;
    br->sysdeps.up = true;
    if(!MSTP_IN_bridge_create(br, br->sysdeps.macaddr))
        die("bridge_create failed");
    s->n_ports = n_ports;
    for(int p = 1; p <= n_ports; ++p)
    {
        port_t *prt = calloc(1, sizeof(port_t));
        prt->bridge = br;
        snprintf(prt->sysdeps.name, IFNAMSIZ, "p%d", p);
        prt->sysdeps.if_index = 1000 * (i + 1) + p;
        memcpy(prt->sysdeps.macaddr, br->sysdeps.macaddr, ETH_ALEN);
        if(!MSTP_IN_port_create_and_add_tail(prt, p))
            die("port_create failed");
        s->ports[p] = prt;

        /* Every link is point to point; no port is an edge port, by configuration or by
         * detection (our switches have only portfast). The cost defaults to 19, as ours. */
        CIST_PortConfig pc;
        memset(&pc, 0, sizeof(pc));
        pc.admin_external_port_path_cost = 19;
        pc.set_admin_external_port_path_cost = true;
        pc.admin_edge_port = false;
        pc.set_admin_edge_port = true;
        pc.auto_edge_port = false;
        pc.set_auto_edge_port = true;
        pc.admin_p2p = p2pForceTrue;
        pc.set_admin_p2p = true;
        if(MSTP_IN_set_cist_port_config(prt, &pc))
            die("port config refused");
    }

    CIST_BridgeConfig bc;
    memset(&bc, 0, sizeof(bc));
    bc.protocol_version = mode;
    bc.set_protocol_version = true;
    if(MSTP_IN_set_cist_bridge_config(br, &bc))
        die("bridge config refused");
    if(MSTP_IN_set_msti_bridge_config(GET_CIST_TREE(br), priority / 4096))
        die("bridge priority refused");
    MSTP_IN_set_bridge_enable(br, true);
    drain();
}

/* ---- the report ---- */

static void print_id(bridge_identifier_t id)
{
    printf("%u.%02x%02x%02x%02x%02x%02x", __be16_to_cpu(id.s.priority), id.s.mac_address[0],
           id.s.mac_address[1], id.s.mac_address[2], id.s.mac_address[3], id.s.mac_address[4],
           id.s.mac_address[5]);
}

static const char *role_name(port_role_t r)
{
    switch(r)
    {
        case roleRoot: return "root";
        case roleDesignated: return "designated";
        case roleAlternate: return "alternate";
        case roleBackup: return "backup";
        case roleMaster: return "master";
        case roleDisabled:
        default: return "disabled";
    }
}

static const char *state_name(int s)
{
    switch(s)
    {
        case BR_STATE_LEARNING: return "learning";
        case BR_STATE_FORWARDING: return "forwarding";
        default: return "discarding"; /* disabled, blocking, listening */
    }
}

static void dump(void)
{
    printf("t=%ld\n", seconds);
    for(int i = 0; i < n_bridges; ++i)
    {
        CIST_BridgeStatus bs;
        MSTP_IN_get_cist_bridge_status(&bridges[i].br, &bs);
        printf("b%d root ", i);
        print_id(bs.designated_root);
        printf(" cost %u rootport ", bs.root_path_cost);
        __u16 rp = __be16_to_cpu(bs.root_port_id);
        if(rp)
            printf("%u", rp & 0x0fff);
        else
            printf("-");
        printf("\n");
        for(int p = 1; p <= bridges[i].n_ports; ++p)
        {
            CIST_PortStatus ps;
            MSTP_IN_get_cist_port_status(bridges[i].ports[p], &ps);
            printf("b%dp%d %s %s\n", i, p, role_name(ps.role), state_name(ps.state));
        }
    }
    fflush(stdout);
}

/* ---- the script ---- */

int main(int argc, char **argv)
{
    if(argc > 1 && !strcmp(argv[1], "-v"))
        log_level = LOG_LEVEL_STATE_MACHINE_TRANSITION;
    queue = calloc(MAX_QUEUE, sizeof(queued_t));
    char line[512];
    while(fgets(line, sizeof(line), stdin))
    {
        ++line_no;
        char *hash = strchr(line, '#');
        if(hash)
            *hash = 0;
        char cmd[32], s[64];
        int a, b, c, d;
        unsigned u;
        if(sscanf(line, "%31s", cmd) != 1)
            continue;
        if(!strcmp(cmd, "mode"))
        {
            if(n_bridges)
                die("mode comes before any bridge");
            if(sscanf(line, "%*s %63s", s) != 1)
                die("mode stp|rstp");
            if(!strcmp(s, "stp"))
                mode = protoSTP;
            else if(!strcmp(s, "rstp"))
                mode = protoRSTP;
            else
                die("mode stp|rstp");
        }
        else if(!strcmp(cmd, "bridge"))
        {
            if(sscanf(line, "%*s %u %63s %d", &u, s, &a) != 3)
                die("bridge <priority> <mac> <ports>");
            add_bridge(u, s, a);
        }
        else if(!strcmp(cmd, "cost"))
        {
            if(sscanf(line, "%*s %d %d %u", &a, &b, &u) != 3)
                die("cost <bridge> <port> <cost>");
            CIST_PortConfig pc;
            memset(&pc, 0, sizeof(pc));
            pc.admin_external_port_path_cost = u;
            pc.set_admin_external_port_path_cost = true;
            if(MSTP_IN_set_cist_port_config(port_of(a, b), &pc))
                die("cost refused");
            drain();
        }
        else if(!strcmp(cmd, "prio"))
        {
            if(sscanf(line, "%*s %d %u", &a, &u) != 2)
                die("prio <bridge> <priority>");
            if(a < 0 || a >= n_bridges)
                die("no bridge %d", a);
            if(u % 4096 || u > 61440)
                die("priority %u is not a multiple of 4096", u);
            if(MSTP_IN_set_msti_bridge_config(GET_CIST_TREE(&bridges[a].br), u / 4096))
                die("bridge priority refused");
            /* mstp.c acts on it at the bridge's next second, as under mstpctl. */
            drain();
        }
        else if(!strcmp(cmd, "portprio"))
        {
            if(sscanf(line, "%*s %d %d %u", &a, &b, &u) != 3)
                die("portprio <bridge> <port> <priority>");
            if(u % 16 || u > 240)
                die("port priority %u is not a multiple of 16", u);
            port_t *prt = port_of(a, b);
            MSTI_PortConfig mc;
            memset(&mc, 0, sizeof(mc));
            mc.port_priority = u / 16;
            mc.set_port_priority = true;
            if(MSTP_IN_set_msti_port_config(GET_CIST_PTP_FROM_PORT(prt), &mc))
                die("port priority refused");
            drain();
        }
        else if(!strcmp(cmd, "link"))
        {
            if(sscanf(line, "%*s %d %d %d %d", &a, &b, &c, &d) != 4)
                die("link <bridge> <port> <bridge> <port>");
            if(n_links == MAX_LINKS)
                die("too many links");
            port_of(a, b);
            port_of(c, d);
            sim_link_t *l = &links[n_links++];
            l->b[0] = a;
            l->p[0] = b;
            l->b[1] = c;
            l->p[1] = d;
            l->up = false;
        }
        else if(!strcmp(cmd, "up") || !strcmp(cmd, "down"))
        {
            if(sscanf(line, "%*s %d", &a) != 1)
                die("%s <link>", cmd);
            set_link(a, !strcmp(cmd, "up"));
        }
        else if(!strcmp(cmd, "run"))
        {
            if(sscanf(line, "%*s %d", &a) != 1 || a < 0)
                die("run <seconds>");
            for(int t = 0; t < a; ++t)
            {
                ++seconds;
                for(int i = 0; i < n_bridges; ++i)
                {
                    MSTP_IN_one_second(&bridges[i].br);
                    drain();
                }
            }
        }
        else if(!strcmp(cmd, "dump"))
            dump();
        else
            die("unknown command %s", cmd);
    }
    return 0;
}
