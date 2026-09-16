import json
import tempfile
import unittest
from pathlib import Path

from .validate import ValidationError, load, validate_document


ROOT = Path(__file__).parent
METADATA = load(ROOT / "command_metadata_v1.json")


def document(command):
    return {
        "schema_version": 1,
        "tutorial_id": "test",
        "title": "Test tutorial",
        "steps": [{
            "id": "step-one",
            "command": command,
            "expect": {"exit_code": 0, "stdout_shape": "json"},
            "network": "denied",
            "credentials": "none",
        }],
    }


class TutorialValidatorTests(unittest.TestCase):
    def test_valid_contract(self):
        validate_document(document(["asb", "capabilities", "--format", "json"]), METADATA)

    def test_unknown_option_fails(self):
        with self.assertRaisesRegex(ValidationError, "do not match"):
            validate_document(document(["asb", "capabilities", "--json"]), METADATA)

    def test_reordered_option_fails(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "capabilities", "json", "--format"]), METADATA)

    def test_secret_and_network_contracts_fail(self):
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "report", "token.json"]), METADATA)
        bad = document(["asb", "doctor"])
        bad["steps"][0]["network"] = "allowed"
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)

    def test_unknown_field_fails(self):
        bad = document(["asb", "doctor"])
        bad["steps"][0]["unexpected"] = True
        with self.assertRaises(ValidationError):
            validate_document(bad, METADATA)

    def test_metadata_option_removal_is_detected(self):
        metadata = json.loads(json.dumps(METADATA))
        metadata["commands"]["capabilities"]["forms"] = [[]]
        with self.assertRaises(ValidationError):
            validate_document(document(["asb", "capabilities", "--format", "json"]), metadata)

    def test_file_loader_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "source.json"
            link = Path(directory) / "link.json"
            source.write_text("{}", encoding="utf-8")
            link.symlink_to(source)
            with self.assertRaises(ValidationError):
                load(link)


if __name__ == "__main__":
    unittest.main()
