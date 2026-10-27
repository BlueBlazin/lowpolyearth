"""The complete additional collection; names must not overlap existing packs."""

GROUPS = {
    'americas': (
        'willis-tower', 'john-hancock-center', 'tribune-tower', 'wrigley-building',
        'marina-city', 'aqua-tower', '432-park-avenue', 'woolworth-building',
        '30-rockefeller-plaza', 'transamerica-pyramid', 'salesforce-tower',
        'torre-reforma',
    ),
    'east_asia': (
        'shanghai-tower', 'shanghai-world-financial-center', 'jin-mao-tower',
        'ping-an-finance-centre', 'guangzhou-ctf-finance-centre',
        'guangzhou-international-finance-center', 'citic-tower',
        'cctv-headquarters', 'two-international-finance-centre',
        'international-commerce-centre', 'bank-of-china-tower',
        'hsbc-main-building', 'lotte-world-tower', '85-sky-tower',
    ),
    'japan_southeast_asia': (
        'abeno-harukas', 'tokyo-metropolitan-government-building',
        'mode-gakuen-cocoon-tower', 'umeda-sky-building', 'mahanakhon',
        'baiyoke-tower-ii', 'the-exchange-106', 'capitaspring',
    ),
    'europe': (
        'tour-montparnasse', 'torre-glories', 'commerzbank-tower', 'messeturm',
        'palace-of-culture-and-science', '20-fenchurch-street',
        'leadenhall-building', 'unicredit-tower',
    ),
    'middle_east_oceania': (
        'kingdom-centre', 'al-faisaliah-centre', 'bahrain-world-trade-center',
        'al-hamra-firdous-tower', 'cayan-tower', 'emirates-towers',
        'eureka-tower', 'one-central-park',
    ),
    'extra': (
        'crown-sydney', 'st-regis-chicago',
        'trump-international-hotel-tower-chicago', 'us-bank-tower',
        'wilshire-grand-center', 'comcast-technology-center',
        'central-park-tower', 'bank-of-america-plaza-atlanta',
        '200-clarendon-street',
    ),
}

NAMES = tuple(name for group in GROUPS.values() for name in group)
assert len(NAMES) == len(set(NAMES)) == 59
