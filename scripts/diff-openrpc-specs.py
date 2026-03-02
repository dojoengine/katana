#!/usr/bin/env python3
"""Diff two versions of the Starknet OpenRPC specifications.

Compares method signatures (name, params, result, errors) between two
versions of the spec and outputs a structured markdown report.
"""

import argparse
import json
import sys


def build_method_map(spec):
    """Build a dict of method_name -> method_definition."""
    return {m["name"]: m for m in spec.get("methods", [])}


def extract_ref_name(obj):
    """Extract the last path segment from a $ref string."""
    if isinstance(obj, dict):
        ref = obj.get("$ref", "")
        return ref.rsplit("/", 1)[-1] if "/" in ref else json.dumps(obj, sort_keys=True)
    return str(obj)


def schema_fingerprint(schema):
    """Return a canonical string representation of a schema for comparison."""
    return json.dumps(schema, sort_keys=True)


def compare_params(old_params, new_params):
    """Compare parameter lists and return a list of change descriptions."""
    changes = []
    old_by_name = {p["name"]: p for p in old_params}
    new_by_name = {p["name"]: p for p in new_params}

    for name in sorted(set(new_by_name) - set(old_by_name)):
        req = new_by_name[name].get("required", True)
        changes.append(f"Parameter `{name}` added (required: `{req}`)")

    for name in sorted(set(old_by_name) - set(new_by_name)):
        changes.append(f"Parameter `{name}` removed")

    for name in sorted(set(old_by_name) & set(new_by_name)):
        old_p, new_p = old_by_name[name], new_by_name[name]
        if old_p.get("required", True) != new_p.get("required", True):
            changes.append(
                f"Parameter `{name}`: required "
                f"`{old_p.get('required', True)}` → `{new_p.get('required', True)}`"
            )
        if schema_fingerprint(old_p.get("schema")) != schema_fingerprint(
            new_p.get("schema")
        ):
            changes.append(f"Parameter `{name}`: schema changed")

    return changes


def compare_result(old_result, new_result):
    """Compare result schemas and return change descriptions."""
    old_schema = schema_fingerprint(old_result.get("schema") if old_result else None)
    new_schema = schema_fingerprint(new_result.get("schema") if new_result else None)
    if old_schema != new_schema:
        return ["Return type changed"]
    return []


def compare_errors(old_errors, new_errors):
    """Compare error lists and return change descriptions."""
    old_set = {schema_fingerprint(e) for e in old_errors}
    new_set = {schema_fingerprint(e) for e in new_errors}
    changes = []
    for e in sorted(new_set - old_set):
        changes.append(f"Error added: `{extract_ref_name(json.loads(e))}`")
    for e in sorted(old_set - new_set):
        changes.append(f"Error removed: `{extract_ref_name(json.loads(e))}`")
    return changes


def format_method_params(method):
    """Format a method's parameter list as a concise string."""
    params = method.get("params", [])
    if not params:
        return "(no params)"
    parts = []
    for p in params:
        req = "required" if p.get("required", True) else "optional"
        parts.append(f"`{p['name']}` ({req})")
    return ", ".join(parts)


def diff_api(old_spec, new_spec):
    """Diff two spec files and return (added, removed, changed, new_map)."""
    old_map = build_method_map(old_spec)
    new_map = build_method_map(new_spec)

    old_names = set(old_map)
    new_names = set(new_map)

    added = sorted(new_names - old_names)
    removed = sorted(old_names - new_names)

    changed = {}
    for name in sorted(old_names & new_names):
        method_changes = []
        method_changes.extend(
            compare_params(
                old_map[name].get("params", []),
                new_map[name].get("params", []),
            )
        )
        method_changes.extend(
            compare_result(
                old_map[name].get("result"),
                new_map[name].get("result"),
            )
        )
        method_changes.extend(
            compare_errors(
                old_map[name].get("errors", []),
                new_map[name].get("errors", []),
            )
        )
        if method_changes:
            changed[name] = method_changes

    return added, removed, changed, new_map


def format_section(title, filename, added, removed, changed, new_map):
    """Format a single API section as a markdown table."""
    lines = [f"### {title} (`{filename}`)", ""]

    if not added and not removed and not changed:
        return [], 0, 0, 0

    lines.append("| Method | Status | Details |")
    lines.append("|--------|--------|---------|")

    for name in added:
        params_str = format_method_params(new_map.get(name, {}))
        lines.append(f"| `{name}` | Added | {params_str} |")

    for name in removed:
        lines.append(f"| `{name}` | Removed | |")

    for name, details in changed.items():
        lines.append(f"| `{name}` | Changed | {'; '.join(details)} |")

    lines.append("")
    return lines, len(added), len(removed), len(changed)


def main():
    parser = argparse.ArgumentParser(description="Diff OpenRPC specs")
    parser.add_argument("--old-read", required=True)
    parser.add_argument("--new-read", required=True)
    parser.add_argument("--old-write", required=True)
    parser.add_argument("--new-write", required=True)
    parser.add_argument("--old-trace", required=True)
    parser.add_argument("--new-trace", required=True)
    parser.add_argument("--old-version", required=True)
    parser.add_argument("--new-version", required=True)
    args = parser.parse_args()

    specs = {}
    for key, attr in [
        ("old-read", "old_read"),
        ("new-read", "new_read"),
        ("old-write", "old_write"),
        ("new-write", "new_write"),
        ("old-trace", "old_trace"),
        ("new-trace", "new_trace"),
    ]:
        path = getattr(args, attr)
        try:
            with open(path) as f:
                specs[key] = json.load(f)
        except (FileNotFoundError, json.JSONDecodeError) as e:
            print(f"Error loading {path}: {e}", file=sys.stderr)
            sys.exit(1)

    output = [f"## OpenRPC Spec Diff: v{args.old_version} → v{args.new_version}", ""]

    api_stats = []

    for title, filename, old_key, new_key in [
        ("Read API", "starknet_api_openrpc.json", "old-read", "new-read"),
        ("Write API", "starknet_write_api.json", "old-write", "new-write"),
        ("Trace API", "starknet_trace_api_openrpc.json", "old-trace", "new-trace"),
    ]:
        added, removed, changed, new_map = diff_api(specs[old_key], specs[new_key])
        lines, a, r, c = format_section(
            title, filename, added, removed, changed, new_map
        )
        output.extend(lines)
        api_stats.append((title, a, r, c))

    total_a = sum(s[1] for s in api_stats)
    total_r = sum(s[2] for s in api_stats)
    total_c = sum(s[3] for s in api_stats)

    output.append("### Summary")
    output.append("")
    output.append("| | Added | Removed | Changed |")
    output.append("|---|---|---|---|")
    for title, a, r, c in api_stats:
        output.append(f"| {title} | {a} | {r} | {c} |")
    output.append(f"| **Total** | **{total_a}** | **{total_r}** | **{total_c}** |")
    output.append("")

    print("\n".join(output))


if __name__ == "__main__":
    main()
