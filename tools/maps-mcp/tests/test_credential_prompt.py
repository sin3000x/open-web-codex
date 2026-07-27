from __future__ import annotations

import asyncio
import unittest
from urllib.parse import urlencode
from urllib.request import Request
from urllib.request import urlopen

from maps_mcp.credential_prompt import LoopbackCredentialPrompt


class LoopbackCredentialPromptTests(unittest.IsolatedAsyncioTestCase):
    async def test_collects_key_out_of_band_and_hides_it_from_page(self) -> None:
        prompt = LoopbackCredentialPrompt()
        prompt.start()
        try:
            page = await asyncio.to_thread(lambda: urlopen(prompt.url).read().decode())
            self.assertIn('type="password"', page)
            self.assertNotIn("secret-value", page)

            body = urlencode(
                {"provider": "mapbox", "api_key": "secret-value", "remember": "yes"}
            ).encode()
            request = Request(prompt.url, data=body, method="POST")
            await asyncio.to_thread(lambda: urlopen(request).read())
            submission = await prompt.wait(timeout_seconds=2)

            self.assertEqual(submission.provider, "mapbox")
            self.assertEqual(submission.api_key, "secret-value")
            self.assertTrue(submission.remember)
        finally:
            await prompt.close()


if __name__ == "__main__":
    unittest.main()
