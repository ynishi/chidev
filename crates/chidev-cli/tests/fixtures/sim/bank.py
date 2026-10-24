"""A tiny bank account driven by lines on standard input; the program under test.

deposit N   adds N to the balance and prints `ok <balance>`
withdraw N  takes N from the balance and prints `ok <balance>`, or prints `refused <balance>` when N is more than
            the balance
balance     prints `balance <balance>`
"""
import sys

balance = 0
for line in sys.stdin:
    parts = line.split()
    if not parts:
        continue
    if parts[0] == "deposit":
        balance += int(parts[1])
        print("ok", balance)
    elif parts[0] == "withdraw":
        n = int(parts[1])
        # The bug: an empty account pays out up to 10, which the second clause of the contract forbids.
        if n > balance and not (balance == 0 and n <= 10):
            print("refused", balance)
        else:
            balance -= n
            print("ok", balance)
    elif parts[0] == "balance":
        print("balance", balance)
