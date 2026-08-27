"""Validate the test-only frontend ledger and authorized typed watch replies."""

import json
import re

from frontend_capture_io import integer, require, validate_project_files
from frontend_capture_services import ServiceAudit


REQUIRED_MEASUREMENTS = {
    "s04-first-row-half-units": {"html_string_len"},
    "s04-lines-width-evaluation": {"html_string_lines"},
    "s04-substring-explicit-break": {"html_substring"},
    "s04-substring-unicode-cuts": {"html_substring"},
    "s04-entity-measurement": {"html_string_len"},
    "s04-substring-lazy-error-frontier": {"html_substring"},
    "s04-style-and-position": {"html_string_len"},
    "s04-missing-image": {"html_string_len"},
    "s04-canvas-two-pixels-revision": {"html_string_len", "sample_canvas_pixel"},
    "s04-full-layout-later-rows": {"html_string_len"},
    "s04-file-two-pixels": {"html_string_len", "sample_canvas_pixel"},
    "s04-length-int32-units": {"html_string_len"},
}


def stable(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def debug_pairs(records):
    pending, pairs = {}, []
    # Browser pumping can observe a reply before the submit acknowledgement
    # publishes its real message ID into the ledger. Pair by IDs, not log order.
    for record in records:
        if record["channel"] != "debug":
            continue
        message = record["message"]
        if record["direction"] == "send" and message["type"] == "request":
            key = integer(record["messageId"])
            require(key not in pending, "duplicate debug request ID")
            pending[key] = record
    for record in records:
        if record["channel"] != "debug":
            continue
        message = record["message"]
        if record["direction"] == "receive" and message["type"] in ("response", "error"):
            key = integer(record["correlationId"])
            require(key in pending, "debug reply without its actual request")
            request = pending.pop(key)
            require(integer(request["epoch"]) == integer(record["epoch"]), "stale debug epoch")
            pairs.append((request, record))
    return pairs


def variable_catalog(pairs, stop):
    """Use the last complete real pagination chain for this exact stop token."""
    chain, complete, expected_cursor = None, None, None
    cursors = set()
    for request, reply in pairs:
        command = request["message"]["value"]["command"]
        if command.get("type") != "list_variables" or command.get("stop") != stop:
            continue
        response = reply["message"]
        require(response["type"] == "response" and response["value"]["type"] == "variable_page",
                "variable listing failed")
        page = response["value"]["value"]
        require(page["stop"] == stop, "variable page stop mismatch")
        cursor = command.get("cursor")
        if cursor is None:
            chain, cursors, expected_cursor = [], set(), None
        require(chain is not None and cursor == expected_cursor, "incomplete variable pagination")
        key = stable(cursor)
        require(key not in cursors and len(cursors) < 256, "repeated/excessive variable cursor")
        cursors.add(key)
        require(isinstance(page["variables"], list) and len(page["variables"]) <= 256,
                "invalid variable page size")
        chain.extend(page["variables"])
        expected_cursor = page.get("next_cursor")
        if expected_cursor is None:
            complete, chain = chain, None
    require(complete is not None, "typed watches lack complete variable pages")
    return complete


def typed_watches(request, inspected, records):
    watches = request.get("watch", [])
    require(isinstance(watches, list) and len(watches) <= 256 and len(set(watches)) == len(watches),
            "invalid manifest watch list")
    if not watches:
        return {}, [], []
    if not isinstance(inspected, dict) or inspected.get("version") != 1:
        return {}, list(watches), []
    stop = inspected["stop"]
    require(set(inspected["values"]) == set(watches), "typed watch set differs from request")
    pairs = debug_pairs(records)
    catalog = variable_catalog(pairs, stop)
    values, missing, provenance = {}, [], []
    for watch in watches:
        parsed = re.fullmatch(r"([A-Za-z_][A-Za-z0-9_]*)(?::([0-9]+(?:,[0-9]+)*))?", watch)
        require(parsed is not None, "unsupported typed watch syntax")
        name, index_string = parsed.groups()
        candidates = [item for item in catalog if item["name"] == name]
        item = inspected["values"][watch]
        require(type(item.get("present")) is bool, "typed watch lacks presence flag")
        if not item["present"] and item.get("error") in ("not_found", "ambiguous"):
            require(len(candidates) == 0 if item["error"] == "not_found" else len(candidates) > 1,
                    "typed watch absence contradicts real variable pages")
            missing.append(watch)
            continue
        require(len(candidates) == 1, "typed watch does not uniquely resolve")
        variable = candidates[0]
        command = item["command"]
        require(command.get("type") == "read_variable" and command.get("stop") == stop,
                "typed watch uses wrong debug command/stop")
        reference = command["value"]
        indices = ([int(value) for value in index_string.split(",")] if index_string else
                   [0] * len(variable["dimensions"]))
        require(reference["symbol_key"] == variable["symbol_key"] and
                reference["storage"] == variable["storage"] and
                [integer(value) for value in reference["indices"]] == indices and
                integer(reference["generation"]) == integer(stop["program_generation"]) and
                all(reference.get(field) is None for field in ("fiber_id", "frame_id", "character")),
                "typed watch reference differs from requested variable/index/generation")
        matches = [(sent, received) for sent, received in pairs
                   if sent["message"]["value"]["command"] == command and
                   received["message"].get("value") == item["response"]]
        require(matches, "typed watch lacks its real correlated debug response")
        sent, received = matches[-1]
        response = item["response"]
        if not item["present"]:
            require(item.get("error") == "unexpected_response" and
                    response.get("type") != "variable_value", "invalid failed typed watch")
            missing.append(watch)
            continue
        require(response.get("type") == "variable_value" and
                response["value"]["reference"] == reference and
                response["value"]["value"] == item["value"],
                "typed watch value differs from actual debug response")
        value = item["value"]
        require(value["type"] == variable["value_kind"], "typed value disagrees with its descriptor kind")
        if value["type"] == "integer":
            require(type(value["value"]) is not int or abs(value["value"]) <= (1 << 53) - 1,
                    "large JS Integer must be captured as an exact decimal string")
            observed = integer(value["value"], -(1 << 63), (1 << 63) - 1)
        elif value["type"] == "string":
            require(isinstance(value["value"], str), "invalid String debug value")
            observed = value["value"]
        elif value["type"] == "boolean":
            require(type(value["value"]) is bool, "invalid Boolean debug value")
            observed = value["value"]
        else:
            missing.append(watch)
            continue
        values[watch] = observed
        provenance.append({"watch": watch, "requestIndex": sent["index"],
                           "responseIndex": received["index"], "stop": stop})
    return values, missing, provenance


class CaseCapture:
    def __init__(self, case, identity, menu_number, payload_hashes):
        require(len(case["requests"]) == 1 and case["requests"][0]["request"].get("op") == "run",
                "frontend menu adapter currently supports one run entry per fresh case")
        self.case, self.identity = case, identity
        self.menu_number = menu_number
        self.payload_hashes = payload_hashes
        self.project_files = None
        self.records, self.record_encodings = [], []
        self.ids = set()
        self.hello = None
        self.load = None
        self.loaded_output = None
        self.epoch = None
        self.services = None
        self.final = None
        self.final_record_count = None
        self.receive_sequence = {"runtime": -1, "debug": -1}
        self.loaded_record_count = 0
        self.start_count = 0

    def snapshot(self, packet):
        require(packet["case"] == self.case["id"], "snapshot case order mismatch")
        snapshot = packet["snapshot"]
        require(snapshot.get("bridgeKind") == self.identity["frontend"],
                "actual snapshot frontend differs from reported provenance")
        identity = snapshot["buildIdentity"]
        require(identity["corePin"] == self.identity["coreSha"], "frontend core pin mismatch")
        if self.identity["frontend"] == "browser":
            require(identity.get("wasmRevision") == self.identity["wasmAssets"]["revision"],
                    "loaded WASM asset revision differs from actual JS/WASM files")
        evidence = snapshot["serviceEvidence"]
        require(evidence.get("version") == 1 and evidence.get("enabled") is True and
                evidence.get("overflow") is False and evidence.get("failure") is None,
                "frontend evidence disabled, overflowed or failed")
        integer(evidence["bytes"], maximum=16 * 1024 * 1024)
        records = evidence["records"]
        require(isinstance(records, list) and len(self.records) <= len(records) <= 8192,
                "truncated or excessive frontend ledger")
        for index, record in enumerate(records):
            require(integer(record["index"]) == index, "frontend ledger index gap")
            encoded = stable(record)
            if index < len(self.records):
                require(encoded == self.record_encodings[index], "frontend ledger history changed")
                continue
            self.accept(record)
            self.records.append(record)
            self.record_encodings.append(encoded)
        require(integer(snapshot["runtimeEpoch"]) == self.epoch,
                "snapshot runtime epoch differs from actual ledger")
        if packet["stage"] == "loaded":
            require(self.loaded_output is None and self.load is not None, "missing/duplicate load observation")
            self.loaded_output = snapshot["output"]
            self.loaded_record_count = len(self.records)
            require(self.start_count == 1, "load lacks exactly one explicit seeded new-game request")
            require(isinstance(self.loaded_output, list) and
                    all(isinstance(line, str) for line in self.loaded_output), "invalid load output")
        elif packet["stage"] == "complete":
            require(self.final is None and packet["request"] == self.case["requests"][0]["request"],
                    "duplicate completion or request mismatch")
            require(self.loaded_output is not None and not self.services.pending,
                    "completion precedes load or leaves an unanswered service request")
            self.final = packet
            self.final_record_count = len(self.records)
        elif packet["stage"] == "identity":
            require(self.project_files is None, "duplicate actual project inventory observation")
            self.project_files = validate_project_files(
                snapshot["lastDownload"]["projectIdentityFiles"], self.payload_hashes,
                [item["path"] for item in self.identity["submittedPayloads"]])
        else:
            require(packet["stage"] == "watchdog", "unknown observation stage")

    def accept(self, record):
        require(record["direction"] in ("receive", "send") and
                record["channel"] in ("runtime", "debug"), "invalid ledger direction/channel")
        message_id = integer(record["messageId"])
        key = (record["direction"], record["channel"], message_id)
        require(key not in self.ids, "duplicate ledger message ID")
        self.ids.add(key)
        if record["direction"] == "receive":
            sequence = integer(record["sequence"])
            require(sequence > self.receive_sequence[record["channel"]],
                    "runtime receive sequence is not increasing")
            self.receive_sequence[record["channel"]] = sequence
        else:
            require("sequence" not in record, "send record invents a receive sequence")
        message = record["message"]
        if record["channel"] != "runtime":
            return
        if message["type"] == "server_hello":
            require(self.hello is None and record["direction"] == "receive", "duplicate/invalid server hello")
            self.hello = message["value"]
            self.epoch = integer(self.hello["epoch"])
            self.services = ServiceAudit(self.hello["selected_capabilities"])
        elif self.epoch is not None:
            epoch = integer(record["epoch"])
            if self.loaded_output is None:
                if record["direction"] == "receive":
                    require(epoch >= self.epoch and (epoch == self.epoch or not self.services.pending),
                            "setup epoch regressed or retired a pending service")
                    self.epoch = epoch
                else:
                    require(epoch <= self.epoch, "setup send claims an unobserved future epoch")
            else:
                # NewGame advances the setup epoch. After the ready observation,
                # lifecycle fault-injection belongs to its own separate audit.
                require(epoch == self.epoch, "case runtime epoch changed")
        if message["type"] == "start" and record["direction"] == "send":
            mode = message["value"]["mode"]
            require(mode.get("type") == "new_game" and integer(mode["seed"]) == self.identity["seed"],
                    "actual new-game seed/mode differs from capture identity")
            self.start_count += 1
        if message["type"] == "project_load_report":
            require(self.load is None and record["direction"] == "receive", "duplicate/invalid load report")
            self.load = message["value"]
            require(self.load.get("success") is True and self.load.get("compatibility") == self.identity["profile"],
                    "fixture load failed or loaded a different compatibility identity")
        if message["type"] in ("service_request", "service_response", "cancel_external_request"):
            require(self.services is not None, "service traffic precedes negotiation")
            self.services.accept(record)

    def finish(self):
        require(self.hello is not None and self.loaded_output is not None and self.final is not None,
                "incomplete real frontend case")
        require(self.project_files is not None, "missing actual exported project source identity")
        require(not self.services.pending, "unanswered service request at case completion")
        # A later identity export cannot fill missing completion observations.
        records = self.records[:self.final_record_count]
        services = [item for item in self.services.completed
                    if item.get("responseIndex", item.get("cancelIndex")) < self.final_record_count]
        inputs = [record["message"]["value"]["intent"] for record in records[self.loaded_record_count:]
                  if record["channel"] == "runtime" and record["direction"] == "send" and
                  record["message"]["type"] == "input"]
        require(inputs == [{"type": "commit_text", "value": str(self.menu_number)}],
                "actual menu input differs from the manifest case number")
        snapshot = self.final["snapshot"]
        output = snapshot["output"]
        require(isinstance(output, list) and all(isinstance(line, str) for line in output) and
                output[:len(self.loaded_output)] == self.loaded_output,
                "operation output lost its exact load prefix")
        operation = output[len(self.loaded_output):]
        hazard = self.case["id"] == "s04-lines-no-progress"
        if hazard:
            require("S04_NO_PROGRESS_READY" in self.loaded_output, "missing hazard ready observation")
            start = 0
        else:
            require(operation.count("S04_ENTRY_BEGIN") == 1, "missing/ambiguous entry start marker")
            start = operation.index("S04_ENTRY_BEGIN") + 1
        fault_events = [record for record in records[self.loaded_record_count:] if record["channel"] == "runtime" and
                        record["direction"] == "receive" and record["message"]["type"] == "fault"]
        faulted = snapshot.get("fault") is not None or snapshot.get("phase") == "faulted"
        if faulted:
            require(fault_events and "S04_CASE_COMPLETE" not in operation, "fault lacks its actual runtime event")
            end = len(operation)
        else:
            require(not hazard, "non-progress hazard unexpectedly completed")
            require(snapshot.get("phase") in ("waiting_input", "debug_paused") and
                    operation.count("S04_CASE_COMPLETE") == 1, "entry did not actually complete")
            end = operation.index("S04_CASE_COMPLETE")
            require(end >= start, "completion precedes entry")
            measured = {item.get("operation") for item in services
                        if item.get("outcome") == "ready" and
                        item["requestIndex"] >= self.loaded_record_count}
            require(REQUIRED_MEASUREMENTS.get(self.case["id"], set()) <= measured,
                    "successful service fixture lacks its actual measurement replies")
        request = self.case["requests"][0]["request"]
        watches, missing, watch_provenance = typed_watches(
            request, self.final.get("inspect"), records[self.loaded_record_count:])
        diagnostics = [record["message"]["value"] for record in records[self.loaded_record_count:]
                       if record["channel"] == "runtime" and record["direction"] == "receive" and
                       record["message"]["type"] in ("fault", "diagnostic")]
        status = "blocked" if missing else "executed"
        step = {"request": request, "status": status,
                "reason": f"unobserved typed watches: {missing}" if missing else None,
                "result": {"ok": not faulted, "termination": "faulted" if faulted else "completed",
                           "output": operation[start:end], "watches": watches,
                           "diagnostics": diagnostics},
                "watchProvenance": watch_provenance,
                "serviceEvidence": services,
                "observation": {"rawOutputStart": len(self.loaded_output) + start,
                                "rawOutputEnd": len(self.loaded_output) + end,
                                "watchdogStatus": "runner_evidence_required"}}
        return {"id": self.case["id"], "group": self.case["group"], "load": self.load,
                "steps": [step], "submittedProjectFiles": self.project_files,
                "frontendCaptureStatus": "observed_not_parity_verdict"}
