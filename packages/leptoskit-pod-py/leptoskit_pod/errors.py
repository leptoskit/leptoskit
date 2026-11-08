"""Pod codec errors matching Rust leptoskit_pod::Error variants."""


class PodError(Exception):
    """Base codec error; subclass names match Rust variant names."""

    code: str = "PodError"


class Truncated(PodError):
    code = "Truncated"

    def __init__(self) -> None:
        super().__init__("truncated pod buffer")


class Trailing(PodError):
    code = "Trailing"

    def __init__(self) -> None:
        super().__init__("trailing bytes after pod value")


class TooLarge(PodError):
    code = "TooLarge"

    def __init__(self) -> None:
        super().__init__("pod value too large")


class InvalidUtf8(PodError):
    code = "InvalidUtf8"

    def __init__(self) -> None:
        super().__init__("invalid utf-8 in pod string")


class InvalidTag(PodError):
    code = "InvalidTag"

    def __init__(self, tag: int) -> None:
        self.tag = tag
        super().__init__(f"invalid pod tag {tag}")


class InvalidEnvelope(PodError):
    code = "InvalidEnvelope"

    def __init__(self) -> None:
        super().__init__("invalid pod envelope")


class UnknownVersion(PodError):
    code = "UnknownVersion"

    def __init__(self, version: int) -> None:
        self.version = version
        super().__init__(f"unknown pod struct version {version}")
