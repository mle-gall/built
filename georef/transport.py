"""Bind survey HTTP transfers to the requesting window's event loop."""
from core.host_callbacks import HostService as Service, Signal
from core.input_types import NATIVE

if NATIVE:
    from concurrent.futures import ThreadPoolExecutor
    from urllib.request import HTTPRedirectHandler, HTTPSHandler, Request, build_opener

    from core.tls import https_context
    from ingetrazo_render import WindowJob

    _workers = ThreadPoolExecutor(max_workers=8, thread_name_prefix="survey")

    class _Redirect(HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            if req.type == "https" and not newurl.lower().startswith("https://"):
                raise ValueError("Refusing to downgrade an HTTPS survey request")
            return super().redirect_request(req, fp, code, msg, headers, newurl)

    def _download(url, agent):
        if not url.lower().startswith(("http://", "https://")):
            raise ValueError("Survey URL must use HTTP or HTTPS")
        opener = build_opener(HTTPSHandler(context=https_context()), _Redirect())
        with opener.open(Request(url, headers={"User-Agent": agent.decode()}), timeout=15) as reply:
            data = reply.read(32 * 1024 * 1024 + 1)
            if len(data) > 32 * 1024 * 1024:
                raise ValueError("Survey response exceeds 32 MiB")
            return data

    def request(owner, url, agent, ready, error):
        return WindowJob.submit(owner, _workers.submit(_download, url, agent), ready, error)
