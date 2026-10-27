"""Complete set of sites owned by the heritage asset generator."""

GROUPS = {
    'castles': (
        'neuschwanstein-castle', 'edinburgh-castle', 'windsor-castle',
        'conwy-castle', 'malbork-castle', 'alcazar-of-segovia', 'alhambra',
        'eltz-castle', 'chillon-castle', 'bran-castle', 'predjama-castle',
        'matsumoto-castle', 'osaka-castle',
    ),
    'fortresses': (
        'hwaseong-fortress', 'amber-fort', 'gwalior-fort', 'kumbhalgarh-fort',
        'red-fort', 'derawar-fort', 'krak-des-chevaliers', 'qaitbay-citadel',
        'fasil-ghebbi', 'castillo-san-felipe-del-morro',
        'castillo-san-felipe-de-barajas', 'citadelle-laferriere',
    ),
    'sacred': (
        'milan-cathedral', 'st-peters-basilica', 'cologne-cathedral',
        'mont-saint-michel-abbey', 'st-pauls-cathedral', 'hagia-sophia',
        'blue-mosque', 'sheikh-zayed-grand-mosque', 'hassan-ii-mosque',
        'great-mosque-of-djenne', 'shah-mosque', 'sultan-qaboos-grand-mosque',
        'las-lajas-sanctuary', 'dohany-street-synagogue',
    ),
    'temples': (
        'borobudur', 'shwedagon-pagoda', 'wat-arun', 'potala-palace',
        'paro-taktsang', 'prambanan', 'brihadisvara-temple',
        'meenakshi-amman-temple', 'konark-sun-temple',
        'swaminarayan-akshardham', 'biete-giyorgis', 'taj-mahal', 'sagrada-familia',
    ),
}
NAMES = tuple(name for names in GROUPS.values() for name in names)
