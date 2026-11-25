# SPDX-License-Identifier: GPL-3.0-or-later
"""The Project search field operates on occurrences, tags and section rows."""
import pytest
from built_core import Matrix4

from built_core.layers import Layer
from tests.test_project_drawing_filters import Project
from tests.test_project_occurrences import _container, _group, _two_planes


def project_with(*groups):
    project = Project()
    project.scene.groups[:] = groups
    project.scene.version += 1
    project.action("toggle", "n:")
    return project


def occurrences(project, query):
    return {name for k, (name, _) in project.action("filter", query).items()
            if k.startswith(("i:", "c:"))}


@pytest.mark.parametrize("query,names", [
    ("*MUR*", {"Muro A", "Muro B"}), ("Muro ?", {"Muro A", "Muro B"}),
    ("vent", {"Ventana"}), ("nope", set()), ("", {"Muro A", "Muro B", "Ventana"}),
])
def test_plain_and_wildcard_names(query, names):
    project = project_with(*(_group(n) for n in ("Muro A", "Muro B", "Ventana")))
    assert occurrences(project, query) == names


@pytest.mark.parametrize("field, query", [("hidden", "is:hidden"), ("locked", "is:locked")])
def test_a_restricted_parent_applies_to_its_descendants(field, query):
    child = _group("Kid")
    parent = _container("Building", [child])
    setattr(parent, field, True)
    project = project_with(parent, _group("Outside"))
    assert occurrences(project, query) == {"Building", "Kid"}
    visible = {"Outside"} if field == "hidden" else {"Building", "Kid", "Outside"}
    assert occurrences(project, "is:visible") == visible


def test_search_finds_a_nested_selection_and_keeps_only_its_path():
    child = _group("Kid")
    parent = _container("Building", [child, _group("Sibling")])
    p = project_with(parent, _group("Outside"))
    p.scene.select([child])
    assert occurrences(p, "is:selected") == {"Building", "Kid"}
    assert occurrences(p, "kid") == {"Building", "Kid"}
    assert occurrences(p, "") == {"Building", "Outside"}  # saved fold survives the search


def test_component_and_image_search():
    chair = _group("Chair")
    chair.xform, chair.component = Matrix4(), True
    poster = _group("Poster")
    poster.billboard = True
    p = project_with(chair, poster, _group("Wall"))
    assert occurrences(p, "is:component") == {"Chair"}
    assert occurrences(p, "is:image") == {"Poster"}
    assert occurrences(p, "") == {"Chair", "Poster", "Wall"}


def test_invert_and_clear_the_whole_filter():
    a, b, window = (_group(n) for n in ("Muro A", "Muro B", "Ventana"))
    a.hidden = window.hidden = True
    p = project_with(a, b, window)
    assert occurrences(p, "is:hidden") == {"Muro A", "Ventana"}
    assert occurrences(p, "-is:hidden") == {"Muro B"}
    assert occurrences(p, "muro is:hidden") == {"Muro A"}
    assert occurrences(p, "! muro is:hidden") == {"Muro B", "Ventana"}
    assert occurrences(p, "") == {"Muro A", "Muro B", "Ventana"}


def test_section_filter_matches_planes_by_name_and_visibility():
    project = project_with(_group("Wall"))
    scene = project.scene
    a, b = _two_planes(scene)
    a.hidden, b.hidden = False, True
    scene.layers.append(Layer("Walls"))
    scene.version += 1
    keys = lambda query: set(project.action("filter", query))
    section = {"section:" + str(p.uid) for p in (a, b)}
    listed = keys("is:section")
    assert section <= listed and not any(k.startswith(("i:", "preview-tag:")) for k in listed)
    assert keys("is:section is:hidden") & section == {"section:" + str(b.uid)}
    assert keys("is:section is:visible") & section == {"section:" + str(a.uid)}
    assert keys("is:section " + b.name.lower()) & section == {"section:" + str(b.uid)}
    assert not keys("-is:section") & section
    assert occurrences(project, "-is:section") == {"Wall"}
    assert "preview-tag:Walls" in keys("")
