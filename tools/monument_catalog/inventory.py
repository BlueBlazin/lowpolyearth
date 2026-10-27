"""The additional thirty-site monument collection."""

GROUPS = {
    'ancient': (
        'stonehenge', 'ahu-tongariki', 'callanish-stones', 'ring-of-brodgar',
        'trajan-column', 'marcus-aurelius-column', 'luxor-obelisk',
        'aksum-obelisk', 'iron-pillar-delhi', 'ashoka-pillar-vaishali',
    ),
    'japanese': (
        'heian-jingu-great-torii', 'meiji-jingu-second-torii', 'oyunohara-great-torii',
        'kirishima-jingu-great-torii', 'hakozaki-first-torii', 'mitsumine-mitsutorii',
        'nezu-jinja-romon', 'yutoku-inari-romon', 'takeo-onsen-romon',
        'usa-jingu-saidaimon',
    ),
    'gateways': (
        'sanchi-north-torana', 'sungnyemun-gate', 'lempuyang-gates-of-heaven',
        'ming-tombs-stone-archway', 'bangkok-giant-swing', 'patuxai',
        'arch-of-titus', 'arch-of-constantine', 'puerta-de-alcala',
        'rua-augusta-arch',
    ),
}
NAMES = tuple(name for group in GROUPS.values() for name in group)
