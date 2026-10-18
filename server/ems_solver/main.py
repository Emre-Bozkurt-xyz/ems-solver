"""Entry point: uvicorn ems_solver.main:app"""

import logging
from pathlib import Path

from dotenv import load_dotenv

from .app import create_app
from .config import Settings

load_dotenv(Path(__file__).resolve().parent.parent / ".env")
logging.basicConfig(level=logging.INFO, format="%(levelname)s %(name)s: %(message)s")

app = create_app(Settings.from_env())
