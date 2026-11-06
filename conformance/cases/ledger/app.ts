/**
 * A wallet ledger: accounts, deposits, transfers between accounts, and an append-only entry log.
 * Stresses multi-row transactions, guards, and declared errors.
 */
import { app, err, t } from "@aleph/sdk";

const a = app("ledger");

const currency = t.enum("USD", "EUR");
const amount = t.int().min(1).max(100_000_000);

const Account = a.entity("Account", {
  key: ["accountId"],
  fields: {
    accountId: t.id().generated(),
    ownerId: t.string().max(64),
    currency,
    balance: t.int().min(0).default(0),
  },
  indexes: { byOwner: ["ownerId"] },
});

const Entry = a.entity("Entry", {
  key: ["accountId", "entryId"],
  fields: {
    accountId: t.id(),
    entryId: t.id().generated(),
    amount: t.int(),
    balanceAfter: t.int().min(0),
    memo: t.string().max(140).optional(),
    at: t.timestamp().defaultNow(),
  },
  indexes: { byTime: ["accountId", "at"] },
});

a.route("POST /accounts")
  .id("openAccount")
  .body({ currency })
  .handle(($, { body }) => Account.insert({ ownerId: $.caller, currency: body.currency }));

a.route("GET /accounts")
  .id("myAccounts")
  .handle(($) => ({
    accounts: Account.query({ index: "byOwner", prefix: [$.caller], limit: 10 }),
  }));

a.route("POST /accounts/:accountId/deposit")
  .id("deposit")
  .params({ accountId: t.id() })
  .body({ amount })
  .errors({ notFound: err.notFound("Account not found") })
  .handle(($, { params, body, errors }) => {
    const account = Account.getOrFail({ accountId: params.accountId }, errors.notFound);
    $.ensure(account.ownerId.eq($.caller), errors.notFound);
    const balance = account.balance.plus(body.amount);
    Entry.insert({
      accountId: account.accountId,
      amount: body.amount,
      balanceAfter: balance,
      memo: "deposit",
    });
    return account.update({ balance });
  });

a.route("POST /transfers")
  .id("transfer")
  .idempotent({ ttl: "24h", required: true })
  .body({ from: t.id(), to: t.id(), amount, memo: t.string().max(140).optional() })
  .errors({
    sameAccount: err.badRequest("Can't transfer to the same account"),
    notFound: err.notFound("Account not found"),
    notOwner: err.forbidden("You don't own the source account"),
    currencyMismatch: err.badRequest("Accounts use different currencies"),
    insufficientFunds: err.unprocessable("Insufficient funds"),
  })
  .handle(($, { body, errors }) => {
    $.ensure(body.from.neq(body.to), errors.sameAccount);
    const from = Account.getOrFail({ accountId: body.from }, errors.notFound);
    const to = Account.getOrFail({ accountId: body.to }, errors.notFound);
    $.ensure(from.ownerId.eq($.caller), errors.notOwner);
    $.ensure(from.currency.eq(to.currency), errors.currencyMismatch);
    $.ensure(from.balance.gte(body.amount), errors.insufficientFunds);

    const fromAfter = from.balance.minus(body.amount);
    const toAfter = to.balance.plus(body.amount);
    from.update({ balance: fromAfter });
    to.update({ balance: toAfter });
    Entry.insert({
      accountId: body.from,
      amount: body.amount.negate(),
      balanceAfter: fromAfter,
      memo: body.memo,
    });
    Entry.insert({
      accountId: body.to,
      amount: body.amount,
      balanceAfter: toAfter,
      memo: body.memo,
    });
    return { from: fromAfter, to: toAfter };
  });

a.route("GET /accounts/:accountId/entries")
  .id("entries")
  .params({ accountId: t.id() })
  .query({ limit: t.int().min(1).max(100).default(20) })
  .errors({ notFound: err.notFound("Account not found") })
  .handle(($, { params, query, errors }) => {
    const account = Account.getOrFail({ accountId: params.accountId }, errors.notFound);
    $.ensure(account.ownerId.eq($.caller), errors.notFound);
    return {
      entries: Entry.query({ prefix: [params.accountId], limit: query.limit, order: "desc" }),
    };
  });

a.route("GET /accounts/:accountId/entries/between")
  .id("entriesBetween")
  .params({ accountId: t.id() })
  .query({ from: t.timestamp(), to: t.timestamp() })
  .errors({ notFound: err.notFound("Account not found") })
  .handle(($, { params, query, errors }) => {
    const account = Account.getOrFail({ accountId: params.accountId }, errors.notFound);
    $.ensure(account.ownerId.eq($.caller), errors.notFound);
    return {
      entries: Entry.query({
        index: "byTime",
        prefix: [params.accountId],
        range: { gte: query.from, lt: query.to },
        limit: 100,
      }),
    };
  });

export default a;
