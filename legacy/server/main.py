from pathlib import Path
from dotenv import load_dotenv

# Load .env before importing modules that read env vars at import time.
load_dotenv(Path(__file__).resolve().parent / ".env")

import os
import traceback
from fastapi import FastAPI, HTTPException, Request
from pydantic import BaseModel, Field

from auth import verify_request
from claude_service import solve_with_claude

MAX_TEXT_LEN = int(os.environ.get("MAX_TEXT_LEN", "20000"))

app = FastAPI(
    title="AI Solver",
    docs_url=None,
    redoc_url=None,
    openapi_url=None,
)


class SolveRequest(BaseModel):
    text: str = Field(..., min_length=1)
    mode: str = Field(default="solution")


@app.get("/healthz")
async def healthz():
    return {"ok": True}


@app.post("/solve")
async def solve(request: Request, body: SolveRequest):
    raw_body = await request.body()

    ok, reason = verify_request(request.headers, raw_body)
    if not ok:
        raise HTTPException(status_code=401, detail=reason)

    text = body.text.strip()
    if not text:
        raise HTTPException(status_code=400, detail="empty text")

    if len(text) > MAX_TEXT_LEN:
        raise HTTPException(status_code=413, detail="text too large")

    try:
        result = solve_with_claude(problem=text, mode=body.mode.strip() or "solution")
        return {
            "ok": True,
            "result": result,
        }
    except Exception as e:
        traceback.print_exc()
        raise HTTPException(status_code=502, detail=f"claude error: {e}")