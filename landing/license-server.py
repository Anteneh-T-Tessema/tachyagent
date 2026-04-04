#!/usr/bin/env python3
"""
Tachy License Server — generates signed license keys.
Run behind nginx/caddy on your server. Stripe webhooks call POST /webhook.

Usage:
    pip install flask stripe
    export STRIPE_SECRET_KEY=sk_live_...
    export STRIPE_WEBHOOK_SECRET=whsec_...
    export TACHY_LICENSE_SECRET=your-secret-here
    export SMTP_HOST=smtp.example.com
    export SMTP_USER=noreply@tachy.dev
    export SMTP_PASS=...
    python license-server.py
"""

import os
import json
import hmac
import hashlib
import base64
import time
import smtplib
from email.mime.text import MIMEText
from flask import Flask, request, jsonify

app = Flask(__name__)

LICENSE_SECRET = os.environ.get("TACHY_LICENSE_SECRET", "tachy-license-secret-v1")
STRIPE_SECRET = os.environ.get("STRIPE_SECRET_KEY", "")
STRIPE_WEBHOOK_SECRET = os.environ.get("STRIPE_WEBHOOK_SECRET", "")
SMTP_HOST = os.environ.get("SMTP_HOST", "")
SMTP_USER = os.environ.get("SMTP_USER", "")
SMTP_PASS = os.environ.get("SMTP_PASS", "")


def generate_license_key(email: str, tier: str = "individual", expires_at: int = 0, machine_id: str = None) -> str:
    """Generate a signed TACHY-<payload>-<signature> license key."""
    payload = json.dumps({
        "email": email,
        "tier": tier,
        "expires_at": expires_at,
        "machine_id": machine_id,
        "issued_at": int(time.time()),
    }, separators=(",", ":"))

    sig = hmac.new(
        LICENSE_SECRET.encode(),
        payload.encode(),
        hashlib.sha256,
    ).digest()

    payload_b64 = base64.b64encode(payload.encode()).decode()
    sig_b64 = base64.b64encode(sig).decode()

    return f"TACHY-{payload_b64}-{sig_b64}"


def send_license_email(email: str, key: str, tier: str):
    """Send the license key to the customer via email."""
    if not SMTP_HOST:
        print(f"[no SMTP] Would email {email}: {key}")
        return

    body = f"""Welcome to Tachy!

Your {tier} license key:

    {key}

To activate, run:

    tachy activate {key}

This key is tied to your email ({email}).
Keep it safe — you can reuse it on any machine.

Questions? Reply to this email or visit https://tachy.dev/support

— The Tachy Team
"""
    msg = MIMEText(body)
    msg["Subject"] = f"Your Tachy {tier.title()} License Key"
    msg["From"] = SMTP_USER
    msg["To"] = email

    with smtplib.SMTP(SMTP_HOST, 587) as server:
        server.starttls()
        server.login(SMTP_USER, SMTP_PASS)
        server.send_message(msg)
    print(f"License emailed to {email}")


# --- Stripe price IDs (set these to your actual Stripe price IDs) ---
PRICE_TO_TIER = {
    "price_individual_monthly": ("individual", 0),      # $29/mo, perpetual while subscribed
    "price_individual_yearly": ("individual", 0),       # $249/yr
    "price_team_monthly": ("team", 0),                  # $99/mo
    "price_team_yearly": ("team", 0),                   # $899/yr
    "price_enterprise": ("enterprise", 0),              # custom
}


@app.route("/webhook", methods=["POST"])
def stripe_webhook():
    """Handle Stripe checkout.session.completed webhook."""
    payload = request.get_data()
    sig_header = request.headers.get("Stripe-Signature", "")

    if STRIPE_SECRET:
        import stripe
        stripe.api_key = STRIPE_SECRET
        try:
            event = stripe.Webhook.construct_event(payload, sig_header, STRIPE_WEBHOOK_SECRET)
        except Exception as e:
            return jsonify({"error": str(e)}), 400
    else:
        event = json.loads(payload)

    if event.get("type") == "checkout.session.completed":
        session = event["data"]["object"]
        email = session.get("customer_email") or session.get("customer_details", {}).get("email", "")
        price_id = ""

        # Extract price ID from line items
        if "line_items" in session:
            items = session["line_items"].get("data", [])
            if items:
                price_id = items[0].get("price", {}).get("id", "")

        tier, expires = PRICE_TO_TIER.get(price_id, ("individual", 0))
        key = generate_license_key(email, tier, expires)

        # Store in a simple append-only log
        with open("licenses.jsonl", "a") as f:
            f.write(json.dumps({
                "email": email,
                "tier": tier,
                "key": key,
                "stripe_session": session.get("id", ""),
                "created_at": int(time.time()),
            }) + "\n")

        send_license_email(email, key, tier)
        print(f"License generated for {email} ({tier})")

    return jsonify({"status": "ok"})


@app.route("/generate", methods=["POST"])
def manual_generate():
    """Manual key generation (admin only — protect with auth in production)."""
    data = request.get_json()
    email = data.get("email", "")
    tier = data.get("tier", "individual")
    if not email:
        return jsonify({"error": "email required"}), 400

    key = generate_license_key(email, tier)
    return jsonify({"key": key, "email": email, "tier": tier})


@app.route("/verify", methods=["POST"])
def verify_key():
    """Verify a license key is valid (optional — for your admin dashboard)."""
    data = request.get_json()
    key = data.get("key", "")
    parts = key.split("-")
    if len(parts) != 3 or parts[0] != "TACHY":
        return jsonify({"valid": False, "error": "invalid format"}), 400

    try:
        payload_bytes = base64.b64decode(parts[1])
        payload_str = payload_bytes.decode()
        expected_sig = hmac.new(LICENSE_SECRET.encode(), payload_str.encode(), hashlib.sha256).digest()
        provided_sig = base64.b64decode(parts[2])

        if hmac.compare_digest(expected_sig, provided_sig):
            license_data = json.loads(payload_str)
            return jsonify({"valid": True, "license": license_data})
        else:
            return jsonify({"valid": False, "error": "signature mismatch"}), 400
    except Exception as e:
        return jsonify({"valid": False, "error": str(e)}), 400


@app.route("/health")
def health():
    return jsonify({"status": "ok", "service": "tachy-license-server"})


if __name__ == "__main__":
    port = int(os.environ.get("PORT", 8080))
    print(f"Tachy License Server on :{port}")
    app.run(host="0.0.0.0", port=port)
