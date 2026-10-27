---
name: omni-rule-doc-conciseness
description: 'Prevents verbose javadoc and redundant documentation. Apply when generating Java code, adding comments, writing javadoc, or reviewing code documentation. Enforces concise docs only for complex business logic, non-obvious constraints, and public API contracts. Invoke when an omni-* skill mandates this rule.'
metadata:
  author: "OMNI AIFC Team <omni-aifc@att.com>"
keywords: [rule, doc, conciseness]
layer: OMNI-RULE
---

# omni-rule-doc-conciseness

> **Type**: OMNI rule sub-skill (atomic). Invoked by parent `omni-*` skills via the `skill` tool when a workflow mandates this rule. Not user-facing — do not route user prompts here.
>
> **Source rule**: content below is the canonical, fully-inlined rule (migrated from the pre-APM `omni-documentation-conciseness-rule` convention) — apply it in full and verbatim.

---

# 📝 OMNI Documentation Conciseness Rule

> **Applies To**: All OMNI microservices
> **Version**: 2.0.0

---

## 🔥 HARD-STOPS

- ✘ **No javadoc for obvious methods** — Getters, setters, simple constructors, trivial methods MUST NOT have javadoc
- ✘ **No redundant `@param`/`@return`** — Don't document what the code already makes obvious
- ✘ **No empty javadoc blocks** — Either meaningful or omit entirely
- ✘ **No signature-as-description** — Javadoc explains **why** and **constraints**, not restates the method name

---

## ✅ DOCUMENT These (Meaningful)

| Category | When to Document | Focus On |
|----------|-----------------|----------|
| Complex business logic | Multi-step algorithms, bidirectional comparisons, state machines | What it handles, edge cases, null treatment |
| Non-obvious constraints | State transitions, validation rules, ordering requirements | Allowed transitions, limits, side effects |
| Public API contracts | Controller endpoints with headers, auth, error codes | HTTP method, required headers, error scenarios |
| Side effects | Methods that trigger pricing, events, or external calls | Idempotency, rate limiting, downstream impact |

---

## ❌ DON'T DOCUMENT These (Obvious)

| Category | Example |
|----------|---------|
| Getters/setters | `getCartId()`, `setQuantity()` |
| Simple constructors | `new CartItem(productId, quantity)` |
| Trivial methods | `isEmpty()`, `size()` |
| Simple delegation | `return cartRepository.save(cart)` |
| Obvious endpoints | `@GetMapping("/{cartId}")` with no special logic |

---

## 💡 Principles

1. Code over comments — Prefer self-documenting names (`hasValidBillingAddress()` over `check()`)
2. Explain "why", not "what" — Business rationale > code restatement
3. Document non-obvious behavior — Side effects, non-idempotency, feature flag gating

---

**Version**: 2.0.0 | **Last Updated**: 2026-05-04 | **Owner**: OMNI Architecture Team
