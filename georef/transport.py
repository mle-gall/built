"""Survey HTTP jobs and callbacks on the requesting window."""
from concurrent.futures import ThreadPoolExecutor
from functools import partialmethod
from urllib.request import HTTPRedirectHandler, HTTPSHandler, Request, build_opener

from ingetrazo_core import (
    SurveyService, SurveySignal as Signal, survey_download as _download,
    survey_redirect, survey_request as request,
)
from ingetrazo_render import WindowJob

_workers = ThreadPoolExecutor(max_workers=8, thread_name_prefix="survey")


class Service(SurveyService):
    __init__ = SurveyService.bind_host


class _Redirect(HTTPRedirectHandler):
    redirect_request = partialmethod(survey_redirect)
