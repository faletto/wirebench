# HSRP and VRRP

**Date:** 2026-11-08. Source: RFC 2281 (HSRPv1), RFC 3768 (VRRPv2), public vendor docs recalled from memory; own tests (crates/engine/tests/fhrp.rs, golden testkit/scenarios/m3b_fhrp.*). No Packet Tracer material.

Implemented: election, preempt, timers, priority tracking (standby track), resign, gratuitous ARP, virtual MACs, routers and switch VLAN interfaces. Left out: GLBP, HSRPv2/IPv6, authentication (not checked), VRRP tracking, preempt delay, multiple VRRP addresses.
