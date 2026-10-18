class ApiError(Exception):
    """An error returned to the client as {"ok": false, "error": {"code", "message"}}.

    `retryable` tells clients whether falling back to another provider makes sense.
    """

    def __init__(self, status: int, code: str, message: str, *, retryable: bool = False):
        super().__init__(message)
        self.status = status
        self.code = code
        self.message = message
        self.retryable = retryable

    def to_dict(self) -> dict:
        return {"code": self.code, "message": self.message, "retryable": self.retryable}
