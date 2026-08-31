# Tikka Protocol Fee Model

## Fee Collection Point

### At Ticket Purchase
- **Formula:** `(ticket_price × protocol_fee_bp + 9999) / 10000` per ticket (ceiling division)
- **Recipient:** Treasury address
- **Payer:** Ticket buyer (contract retains less of the total price)
- **Rounding Rule:** Always round up in the protocol's favor. Any dust is absorbed by the payer.
- **Example:** 2.5% fee on 100 XLM ticket = 2.5 XLM to treasury, 97.5 XLM to contract

Prize claims do not currently charge a protocol fee. The `platform_fee` field
in `PrizeClaimed` is therefore zero in the implemented claim path.

### Tier Prize Allocation
- **Formula:** `prize_amount × tier_basis_points / 10000` for every tier except the final tier.
- **Final tier:** Receives `prize_amount` minus the amounts allocated to all earlier tiers.
- **Rounding Rule:** Integer-division dust is assigned to the final tier, so all tier prizes sum exactly to `prize_amount` and no prize funds remain undistributed.

## Effective Total Fee

For a raffle with protocol_fee_bp = 250 (2.5%), ticket_price = 100 XLM, and 10 tickets:

- Ticket fees: 10 × 2.5 XLM = 25 XLM
- Prize claim fee: 0 XLM
- **Total protocol revenue: 25 XLM**
