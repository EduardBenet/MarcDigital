from setuptools import setup, find_packages

with open('requirements.txt') as f:
    required = f.read().splitlines()

setup(
    name='MarcDigital',
    version='0.1',
    packages=find_packages(),
    install_requires=[
        required
    ],
    author='Eduard Benet',
    author_email='ebenetcerda@gmail.com',
    description='Digital frame that syncs a Google Photos album',
    long_description=open('README.md').read(),
    long_description_content_type='text/markdown',
    license='MIT',
    url='https://github.com/EduardBenet/MarcDigital',
    classifiers=[
        # Choose your license here, ensure it's consistent with LICENSE.txt
        'License :: OSI Approved :: MIT License',
        'Programming Language :: Python :: 3',
        'Programming Language :: Python :: 3.6',
        'Programming Language :: Python :: 3.7',
        'Programming Language :: Python :: 3.8',
    ],
)
