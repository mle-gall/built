"""Bind survey HTTP transfers to the requesting window's event loop."""
from core.host_callbacks import HostService as Service, Signal
from core.input_types import NATIVE

if NATIVE:
    from concurrent.futures import ThreadPoolExecutor
    from urllib.request import HTTPRedirectHandler, HTTPSHandler, Request, build_opener

    from core.tls import https_context
    from ingetrazo_render import WindowJob

    _workers = ThreadPoolExecutor(max_workers=8, thread_name_prefix="survey")

    from functools import partialmethod
    from ingetrazo_core import survey_download as _download, survey_redirect, survey_request as request

    class _Redirect(HTTPRedirectHandler):
        redirect_request = partialmethod(survey_redirect)
