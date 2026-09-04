"""Check actual service CBOR and negotiation; never implement measurements."""

from frontend_capture_io import decode_payload, integer, require


S04 = {
    ("presentation_query", "html_string_len"): (2, 0),
    ("presentation_query", "html_substring"): (2, 0),
    ("presentation_query", "html_string_lines"): (2, 0),
    ("input_state", "pointer_state"): (1, 0),
    ("canvas", "sample_canvas_pixel"): (1, 0),
}


def version(value):
    require(isinstance(value, dict) and set(value) == {"major", "minor"},
            "invalid operation version")
    return integer(value["major"], maximum=65535), integer(value["minor"], maximum=65535)


def negotiated(capabilities):
    result = {}
    require(isinstance(capabilities, dict) and isinstance(capabilities.get("services"), list),
            "missing actual selected capabilities")
    for item in capabilities["services"]:
        key = (item["kind"], item["operation"])
        require(key not in result, "duplicate negotiated service")
        minimum, maximum = version(item["versions"]["minimum"]), version(item["versions"]["maximum"])
        require(minimum <= maximum and minimum[0] == maximum[0], "invalid service version range")
        if key in S04:
            require(minimum == maximum == S04[key], "wrong negotiated S04 operation version")
        result[key] = (minimum, maximum)
    return result


def field(value, key):
    require(isinstance(value, dict) and key in value, f"missing CBOR map field {key}")
    return value[key]


def context(value):
    require(isinstance(value, dict) and set(value) == {0, 1, 2}, "invalid projection context")
    return tuple(integer(value[key]) for key in range(3))


def variant(value):
    require(isinstance(value, list) and len(value) == 2 and isinstance(value[1], list),
            "invalid minicbor enum representation")
    return integer(value[0], maximum=255), value[1]


def text_at(document, path):
    nodes = field(document, 0)
    require(isinstance(path, list) and 0 < len(path) <= 65, "invalid HTML text-node path")
    for offset, part in enumerate(path):
        require(isinstance(nodes, list), "invalid HTML children")
        index = integer(part, maximum=4095)
        require(index < len(nodes), "HTML text-node path outside document")
        kind, fields = variant(nodes[index])
        if offset == len(path) - 1:
            require(kind == 0 and len(fields) == 3 and isinstance(fields[0], str),
                    "HTML cut does not name a Text node")
            return fields[0]
        require(kind == 1 and len(fields) == 7, "HTML text path traverses a non-element")
        nodes = fields[2]
    raise ValueError("empty HTML text path")


def legal_cut(document, cut):
    text = text_at(document, field(cut, 1))
    byte = integer(field(cut, 2), maximum=4 * 65536)
    units = integer(field(cut, 3), maximum=2 * 65536)
    encoded = text.encode("utf-8")
    require(byte <= len(encoded), "HTML UTF-8 cut outside text")
    try:
        prefix = encoded[:byte].decode("utf-8")
    except UnicodeError as error:
        raise ValueError("HTML cut splits a Unicode scalar") from error
    require(len(prefix.encode("utf-16-le")) // 2 == units, "HTML UTF-8/UTF-16 cut mismatch")


def html_reply(request, reply):
    require(context(field(request, 0)) == context(field(reply, 0)), "stale HTML projection")
    queries, results = field(request, 2), field(reply, 1)
    require(isinstance(queries, list) and 0 < len(queries) <= 128 and
            isinstance(results, list) and len(results) == len(queries),
            "invalid HTML probe count")
    requested_ids = [integer(field(query, 0), maximum=(1 << 32) - 1) for query in queries]
    returned_ids = [integer(field(item, 0), maximum=(1 << 32) - 1) for item in results]
    require(len(set(requested_ids)) == len(requested_ids) and returned_ids == requested_ids,
            "HTML probe ID/order mismatch")
    for query, item in zip(queries, results):
        mode = integer(field(query, 2), maximum=2)
        cuts = field(query, 3)
        require(isinstance(cuts, list) and len(cuts) <= 2048, "HTML cut limit exceeded")
        cut_ids = [integer(field(cut, 0), maximum=(1 << 32) - 1) for cut in cuts]
        require(len(cut_ids) == len(set(cut_ids)), "duplicate HTML cut ID")
        for cut in cuts:
            legal_cut(field(query, 1), cut)
        tag, values = variant(field(item, 1))
        if tag == 1:
            require(len(values) == 1, "invalid HTML probe error")
            error_record(values[0], cbor=True)
            continue
        if mode == 0:
            require(tag == 0 and len(values) == 2, "HTML TextPart reply mode mismatch")
            integer(values[0], -(1 << 63), (1 << 63) - 1)
            require(isinstance(values[1], list), "invalid HTML advances")
            require([integer(field(cut, 0)) for cut in values[1]] == cut_ids,
                    "HTML cut advance ID/order mismatch")
            for cut in values[1]:
                integer(field(cut, 1), -(1 << 63), (1 << 63) - 1)
        elif mode == 1:
            require(not cuts and field(query, 4) is not None, "ImageSlot missing fallback document")
            if tag == 2:
                require(len(values) == 2, "invalid loaded-image dimensions")
                integer(values[0], 1, 8192)
                integer(values[1], 1, 8192)
            else:
                require(tag == 3 and len(values) == 1, "ImageSlot reply mode mismatch")
                integer(values[0], -(1 << 63), (1 << 63) - 1)
        else:
            require(not cuts and tag == 4 and not values, "FixedSlot reply mode mismatch")


def error_record(value, cbor=False):
    code = field(value, 0) if cbor else value.get("code")
    message = field(value, 1) if cbor else value.get("message")
    require(isinstance(code, str) and code and isinstance(message, str), "invalid service error")


def check_pair(request, response):
    operation = request["operation"]
    query = decode_payload(request["payload"])
    result = response["result"]
    if result.get("type") == "error":
        error_record(result["error"])
        return "error"
    require(result.get("type") == "ready", "invalid service result")
    reply = decode_payload(result["payload"])
    if operation in {"html_string_len", "html_substring", "html_string_lines"}:
        html_reply(query, reply)
    elif operation == "pointer_state":
        require(context(query) == tuple(integer(field(reply, i)) for i in (3, 4, 5)),
                "stale pointer projection")
        integer(field(reply, 0), -(1 << 63), (1 << 63) - 1)
        integer(field(reply, 1), -(1 << 63), (1 << 63) - 1)
        require(isinstance(field(reply, 2), str), "pointer button value is not String")
    elif operation == "sample_canvas_pixel":
        require(context(field(query, 0)) == context(field(reply, 0)), "stale canvas projection")
        require(integer(field(query, 2)) == integer(field(reply, 1)), "stale canvas revision")
        integer(field(reply, 2), maximum=(1 << 32) - 1)
    return "ready"


class ServiceAudit:
    def __init__(self, capabilities):
        self.capabilities = negotiated(capabilities)
        self.pending = {}
        self.seen = set()
        self.completed = []

    def accept(self, record):
        message = record["message"]
        value = message.get("value", {})
        if message["type"] == "service_request":
            require(record["direction"] == "receive", "service request direction mismatch")
            request_id = integer(value["request_id"])
            require(request_id not in self.seen, "reused service request ID")
            self.seen.add(request_id)
            key = (value["kind"], value["operation"])
            require(key in self.capabilities, "service request was not negotiated")
            selected = version(value["operation_version"])
            minimum, maximum = self.capabilities[key]
            require(minimum <= selected <= maximum, "service request version not negotiated")
            if key in S04:
                require(selected == S04[key], "wrong S04 service version")
            decode_payload(value["payload"])
            require(len(self.pending) < 128, "too many pending service requests")
            self.pending[request_id] = record
        elif message["type"] == "service_response":
            require(record["direction"] == "send", "service response direction mismatch")
            request_id = integer(value["request_id"])
            require(request_id in self.pending, "unmatched or duplicate service reply")
            request = self.pending.pop(request_id)
            require(integer(record["epoch"]) == integer(request["epoch"]), "stale service epoch")
            left, right = record.get("correlationId"), request.get("correlationId")
            require((None if left is None else integer(left)) ==
                    (None if right is None else integer(right)),
                    "service correlation ID mismatch")
            outcome = check_pair(request["message"]["value"], value)
            self.completed.append({"requestIndex": request["index"],
                                   "responseIndex": record["index"],
                                   "kind": request["message"]["value"]["kind"],
                                   "operation": request["message"]["value"]["operation"],
                                   "version": request["message"]["value"]["operation_version"],
                                   "outcome": outcome})
        elif message["type"] == "cancel_external_request" and value.get("kind") == "service":
            request_id = integer(value["request_id"])
            require(request_id in self.pending, "cancelled unknown service request")
            request = self.pending.pop(request_id)
            self.completed.append({"requestIndex": request["index"],
                                   "cancelIndex": record["index"], "outcome": "cancelled"})
